//! Read-only Titanium item-link inspection; never equips, trades, or activates items.
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// Displayable numeric property from the server's item definition.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ItemStat {
    /// Stable property label.
    pub label: String,
    /// Signed value, preserving penalties.
    pub value: i32,
}
/// Raw attribute and resource modifiers from a static item definition.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ItemBonuses {
    /// Raw strength modifier, before level restrictions and scaling.
    pub strength: i32,
    /// Raw stamina attribute modifier, not endurance.
    pub stamina: i32,
    /// Raw agility modifier.
    pub agility: i32,
    /// Raw dexterity modifier.
    pub dexterity: i32,
    /// Raw charisma modifier.
    pub charisma: i32,
    /// Raw intelligence modifier.
    pub intelligence: i32,
    /// Raw wisdom modifier.
    pub wisdom: i32,
    /// Raw HP capacity modifier.
    pub hit_points: i32,
    /// Raw mana capacity modifier.
    pub mana: i32,
    /// Raw endurance capacity modifier.
    pub endurance: i32,
}

/// Static restrictions and passive spell metadata needed when applying equipment bonuses.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct EquipmentRules {
    /// Minimum character level for this item's benefits.
    pub required_level: u32,
    /// Level below which a server may scale raw bonuses; zero means unspecified.
    pub recommended_level: u32,
    /// Passive effect metadata, absent when the definition has no worn spell.
    pub worn: Option<WornEffect>,
}

/// Server-supplied worn spell, retaining its type and both level fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct WornEffect {
    /// Spell ID resolved against the user's spell file.
    pub spell_id: u32,
    /// Raw item effect type; unknown types must not silently become passive effects.
    pub effect_type: u32,
    /// Worn effect's Level field, kept distinct from Level2.
    pub level: u32,
    /// Worn effect's Level2 field; interpretation depends on server rules.
    pub level2: u32,
}

/// An inspected item; augment/evolution fields are deliberately not surfaced.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ItemDetails {
    /// None means equipment restrictions/passive effects were not decoded.
    pub equipment: Option<EquipmentRules>,
    /// Raw numeric modifiers; None means the definition has not supplied them.
    /// Effective equipment bonuses also depend on level and worn spell effects.
    pub bonuses: Option<ItemBonuses>,
    /// Server item ID.
    pub id: u32,
    /// Display name.
    pub name: String,
    /// Lore description.
    pub lore: String,
    /// Weight in tenths, without floating-point conversion.
    pub weight_tenths: u32,
    /// Equipment slot mask.
    pub slots: u32,
    /// Allowed class mask.
    pub classes: u32,
    /// Allowed race mask.
    pub races: u32,
    /// Applicable flags such as MAGIC and NO DROP.
    pub flags: Vec<String>,
    /// Nonzero supported statistics and effect IDs.
    pub stats: Vec<ItemStat>,
    /// The item's base value in copper, from which merchants price it; None
    /// where a generation's record is not checked.
    pub price: Option<u32>,
    /// The item's picture in the installed UI's item sheets; None where a
    /// generation's record is not checked.
    pub icon: Option<u32>,
}

impl ItemDetails {
    /// Whether the item changes whoever wears, eats or uses it: an
    /// attribute, resistance, capacity, armor class, regeneration, haste or
    /// spell effect. Level requirements and weapon properties change nothing.
    #[must_use]
    pub fn has_modifiers(&self) -> bool {
        const PROPERTIES: [&str; 5] = [
            "Required level",
            "Recommended level",
            "Delay",
            "Range",
            "Damage",
        ];
        self.bonuses
            .is_some_and(|bonuses| bonuses != ItemBonuses::default())
            || self
                .stats
                .iter()
                .any(|stat| !PROPERTIES.contains(&stat.label.as_str()))
    }
}

/// Build a 44-byte inspection request from the preserved 45-hex-digit link body.
///
/// # Errors
/// Rejects malformed links and the reserved quest say-link ID, which can send chat.
pub fn request(link: &str) -> Result<[u8; 44]> {
    ensure!(
        link.len() == 45 && link.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid Titanium item link"
    );
    let number = |start, end| u32::from_str_radix(&link[start..end], 16);
    let id = number(1, 6)?;
    ensure!(id != 0 && id != 0xfffff, "not an inspectable item link");
    let mut body = [0; 44];
    body[..4].copy_from_slice(&id.to_le_bytes());
    for i in 0..5 {
        body[4 + i * 4..8 + i * 4].copy_from_slice(&number(6 + i * 5, 11 + i * 5)?.to_le_bytes());
    }
    body[24..28].copy_from_slice(&number(37, 45)?.to_le_bytes());
    Ok(body)
}

/// Decode the root item in an `ItemPacketViewLink` response, using Titanium field order.
///
/// # Errors
/// Rejects other packet types, oversized/truncated definitions and invalid required fields.
pub fn response(body: &[u8]) -> Result<ItemDetails> {
    ensure!(
        body.len() >= 5 && body.len() <= 64 * 1024,
        "invalid item response length"
    );
    ensure!(body[..4] == [0; 4], "not an item-link view response");
    let text = std::str::from_utf8(&body[4..]).context("item definition is not UTF-8")?;
    let (instance, tail) = text.split_once('"').context("missing item definition")?;
    ensure!(
        instance.bytes().filter(|b| *b == b'|').count() == 11,
        "invalid item instance header"
    );
    let end = tail
        .find("\"|")
        .or_else(|| tail.find("\"\0"))
        .context("unterminated item definition")?;
    let fields: Vec<_> = tail[..end].split('|').collect();
    ensure!(fields.len() >= 159, "truncated Titanium item fields");
    definition(&fields)
}

/// Projects shared Titanium static fields for both link views and inventory instances.
pub(crate) fn definition(fields: &[&str]) -> Result<ItemDetails> {
    ensure!(fields.len() >= 159, "truncated Titanium item fields");
    let uint =
        |index: usize| -> Result<u32> { fields[index].parse().context("invalid item number") };
    let mut flags = Vec::new();
    for (index, zero, name) in [
        (6, true, "NO RENT"),
        (7, true, "NO DROP"),
        (38, false, "MAGIC"),
    ] {
        if (uint(index)? == 0) == zero {
            flags.push(name.into());
        }
    }
    if fields[2].starts_with('*') {
        flags.push("LORE".into());
    }
    let mut stats = Vec::new();
    for (index, label) in [
        (16, "Cold"),
        (17, "Disease"),
        (18, "Poison"),
        (19, "Magic resist"),
        (20, "Fire"),
        (21, "STR"),
        (22, "STA"),
        (23, "AGI"),
        (24, "DEX"),
        (25, "CHA"),
        (26, "INT"),
        (27, "WIS"),
        (28, "HP"),
        (29, "Mana"),
        (30, "AC"),
        (40, "Required level"),
        (44, "Delay"),
        (45, "Recommended level"),
        (49, "Range"),
        (50, "Damage"),
        (114, "HP regen"),
        (115, "Mana regen"),
        (117, "Haste"),
        (134, "Click spell"),
        (139, "Proc spell"),
        (144, "Worn spell"),
        (149, "Focus spell"),
        (154, "Scroll spell"),
    ] {
        let value: i32 = fields[index].parse().context("invalid item stat")?;
        if value != 0 && !(index >= 134 && value == -1) {
            stats.push(ItemStat {
                label: label.into(),
                value,
            });
        }
    }
    let id = uint(4)?;
    ensure!(id != 0 && !fields[1].is_empty(), "missing item identity");
    Ok(ItemDetails {
        equipment: Some(equipment_rules(fields)?),
        bonuses: Some(bonuses(fields)?),
        id,
        name: fields[1].into(),
        lore: fields[2].into(),
        weight_tenths: uint(5)?,
        slots: uint(9)?,
        classes: uint(52)?,
        races: uint(53)?,
        flags,
        stats,
        price: Some(uint(10)?),
        icon: Some(uint(11)?),
    })
}

/// Reads passive-effect metadata without interpreting its server-specific activation rules.
fn equipment_rules(fields: &[&str]) -> Result<EquipmentRules> {
    let value = |index: usize| {
        fields[index]
            .parse::<u32>()
            .context("invalid equipment rule")
    };
    let spell = fields[144].parse::<i32>().context("invalid worn spell")?;
    ensure!(spell >= -1, "invalid worn spell sentinel");
    // Validate all supplied numeric fields even when the spell sentinel denotes no effect.
    let effect_type = value(145)?;
    let level2 = value(146)?;
    let level = value(147)?;
    Ok(EquipmentRules {
        required_level: value(40)?,
        recommended_level: value(45)?,
        worn: if spell > 0 {
            Some(WornEffect {
                spell_id: u32::try_from(spell)?,
                effect_type,
                level,
                level2,
            })
        } else {
            None
        },
    })
}

/// Decodes modifiers independently of display labels, preserving zero and penalties.
fn bonuses(fields: &[&str]) -> Result<ItemBonuses> {
    let value = |index: usize| fields[index].parse::<i32>().context("invalid item bonus");
    Ok(ItemBonuses {
        strength: value(21)?,
        stamina: value(22)?,
        agility: value(23)?,
        dexterity: value(24)?,
        charisma: value(25)?,
        intelligence: value(26)?,
        wisdom: value(27)?,
        hit_points: value(28)?,
        mana: value(29)?,
        endurance: value(111)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modifiers_are_what_an_item_changes_not_what_it_requires() {
        let mut fields = vec!["0"; 159];
        fields[1] = "Synthetic ration";
        fields[4] = "42";
        fields[40] = "5";
        fields[134] = "-1";
        let plain = definition(&fields).unwrap();
        assert!(!plain.has_modifiers(), "{:?}", plain.stats);
        for (index, value) in [(25, "1"), (16, "-2"), (111, "5"), (134, "7"), (117, "3")] {
            let mut fields = fields.clone();
            fields[index] = value;
            assert!(
                definition(&fields).unwrap().has_modifiers(),
                "field {index}"
            );
        }
    }

    #[test]
    fn equipment_metadata_distinguishes_levels_and_validates_effect_sentinels() {
        let mut fields = vec!["0"; 159];
        fields[1] = "Synthetic equipment";
        fields[4] = "42";
        fields[40] = "20";
        fields[45] = "35";
        fields[144] = "123";
        fields[145] = "2";
        fields[146] = "25";
        fields[147] = "30";
        assert_eq!(
            definition(&fields).unwrap().equipment,
            Some(EquipmentRules {
                required_level: 20,
                recommended_level: 35,
                worn: Some(WornEffect {
                    spell_id: 123,
                    effect_type: 2,
                    level: 30,
                    level2: 25
                }),
            })
        );
        for sentinel in ["0", "-1"] {
            fields[144] = sentinel;
            assert!(definition(&fields)
                .unwrap()
                .equipment
                .unwrap()
                .worn
                .is_none());
        }
        fields[144] = "-2";
        assert!(definition(&fields).is_err());
        fields[144] = "0";
        for index in [40, 45, 145, 146, 147] {
            let old = fields[index];
            fields[index] = "bad";
            assert!(definition(&fields).is_err());
            fields[index] = old;
        }
    }
    #[test]
    fn typed_bonuses_preserve_zero_penalties_and_distinct_resource_fields() {
        let mut fields = vec!["0"; 160];
        fields[1] = "Synthetic equipment";
        fields[4] = "123";
        for (index, value) in [
            (21, "-3"),
            (22, "4"),
            (23, "5"),
            (24, "6"),
            (25, "7"),
            (26, "8"),
            (27, "9"),
            (28, "10"),
            (29, "0"),
            (111, "-11"),
        ] {
            fields[index] = value;
        }
        assert_eq!(
            definition(&fields).unwrap().bonuses,
            Some(ItemBonuses {
                strength: -3,
                stamina: 4,
                agility: 5,
                dexterity: 6,
                charisma: 7,
                intelligence: 8,
                wisdom: 9,
                hit_points: 10,
                mana: 0,
                endurance: -11,
            })
        );
        fields[111] = "invalid";
        assert!(definition(&fields).is_err());
        fields[111] = "2147483648";
        assert!(definition(&fields).is_err());
        assert!(definition(&fields[..111]).is_err());
    }
    #[test]
    fn inspection_preserves_hash_and_rejects_quest_actions() {
        let link = format!("0{:05X}{}1234ABCD", 42, "0".repeat(31));
        let body = request(&link).unwrap();
        assert_eq!(&body[..4], &42u32.to_le_bytes());
        assert_eq!(&body[24..28], &0x1234_abcd_u32.to_le_bytes());
        assert!(request(&format!("0FFFFF{}", "0".repeat(39))).is_err());
        assert!(request("broken").is_err());
    }
    #[test]
    fn item_stats_preserve_negative_values_and_reject_truncation() {
        let mut fields = vec!["0".to_owned(); 160];
        fields[1] = "Synthetic blade".into();
        fields[4] = "42".into();
        fields[5] = "25".into();
        fields[10] = "1250".into();
        fields[11] = "640".into();
        fields[21] = "-3".into();
        fields[50] = "9".into();
        let mut body = vec![0; 4];
        body.extend(format!("{}\"{}\"||||||||||\0", "0|".repeat(11), fields.join("|")).bytes());
        let item = response(&body).unwrap();
        assert_eq!(item.id, 42);
        assert!(item.stats.iter().any(|s| s.label == "STR" && s.value == -3));
        // A linked item carries its price and its picture too.
        assert_eq!((item.price, item.icon), (Some(1250), Some(640)));
        assert!(response(&body[..40]).is_err());
        body[0] = 0x69;
        assert!(response(&body).is_err());
    }
}
