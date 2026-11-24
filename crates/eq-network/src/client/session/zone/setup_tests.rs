//! Zone feature construction from synthetic connection preferences, without sockets.
use super::*;
use crate::client::{ClientConfig, ServerProtocol};

const PROTOCOLS: [ServerProtocol; 5] = [
    ServerProtocol::Takp,
    ServerProtocol::EqEmu,
    ServerProtocol::Project1999,
    ServerProtocol::Quarm,
    ServerProtocol::EqEmuRof2,
];

fn config(protocol: ServerProtocol, local_only: bool) -> ClientConfig {
    let mut config = ClientConfig::for_protocol(
        protocol,
        "TestAccount",
        "TestPassword",
        "TestServer",
        "Tester",
    );
    config.local_only = local_only;
    config
}

fn from_config(config: &ClientConfig) -> (World, Features) {
    let world = World::for_zone(7, config);
    let setup = zone_setup(config);
    let features = Features::from_setup(servers::server_type(config.protocol), &setup);
    (world, features)
}

/// The setup used before connection constraints were separated from fixture permission.
fn previous_features(config: &ClientConfig, world: &World) -> Features {
    let mut setup = servers::Setup::new(&config.character, config.auto_eat);
    setup.local_only = world.local_eqemu();
    Features::from_setup(servers::server_type(config.protocol), &setup)
}

fn feature_lists(features: &Features) -> Vec<Vec<Capability>> {
    features
        .provided
        .iter()
        .map(|feature| feature.capabilities())
        .collect()
}

fn assert_same_lists(actual: &Features, previous: &Features, protocol: ServerProtocol) {
    assert_eq!(
        feature_lists(actual),
        feature_lists(previous),
        "{protocol:?}"
    );
    assert_eq!(
        actual.capabilities(),
        previous.capabilities(),
        "{protocol:?}"
    );
    assert_eq!(actual.choices(), previous.choices(), "{protocol:?}");
}

#[test]
fn zone_setup_lists_aa_only_for_constrained_eqemu_and_takp() {
    for local_only in [false, true] {
        for protocol in PROTOCOLS {
            let config = config(protocol, local_only);
            let (world, features) = from_config(&config);
            let expected =
                local_only && matches!(protocol, ServerProtocol::EqEmu | ServerProtocol::Takp);
            assert_eq!(
                features
                    .capabilities()
                    .contains(&Capability::AlternateAdvancement),
                expected,
                "{protocol:?} local {local_only}"
            );
            assert_eq!(
                world.local_eqemu(),
                local_only && protocol == ServerProtocol::EqEmu,
                "fixture permission for {protocol:?} local {local_only}"
            );
        }
    }
}

#[test]
fn zone_setup_keeps_public_feature_lists_unchanged() {
    for protocol in PROTOCOLS {
        let config = config(protocol, false);
        let (world, features) = from_config(&config);
        let previous = previous_features(&config, &world);
        assert_same_lists(&features, &previous, protocol);
    }
}

#[test]
fn zone_setup_changes_only_the_local_takp_aa_feature() {
    for protocol in PROTOCOLS {
        let config = config(protocol, true);
        let (world, features) = from_config(&config);
        let previous = previous_features(&config, &world);
        if protocol != ServerProtocol::Takp {
            assert_same_lists(&features, &previous, protocol);
            continue;
        }
        let mut capabilities = features.capabilities();
        capabilities.retain(|capability| *capability != Capability::AlternateAdvancement);
        assert_eq!(capabilities, previous.capabilities());
        let mut provided = feature_lists(&features);
        provided.retain(|capabilities| capabilities != &[Capability::AlternateAdvancement]);
        assert_eq!(provided, feature_lists(&previous));
        assert_eq!(features.choices(), previous.choices());
        assert!(!world.local_eqemu());
    }
}
