# Contributing

Changes should preserve the boundary between transport framing, login packets,
game packets, and session orchestration. Put a new action's byte layout in
`eq-network-game` and expose it through a typed high-level command or method.
Keep application policy, storage, user-interface behavior, and server-specific
credentials outside this workspace.

Public APIs require rustdoc that explains their purpose and, where applicable,
their errors, panics, and safety requirements. Prefer domain structs and enums
over unlabelled tuples, booleans, or raw opcodes. Mark public data types
`#[non_exhaustive]` when compatible variants or fields may be added. Preserve
unknown wire values when doing so helps diagnostics or forward compatibility.

Tests must use synthetic packet data. Never commit packet captures, live logs,
account names, passwords, tokens, or environment files.

Run the same checks as CI before opening a pull request:

```shell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo test --workspace --all-targets --all-features
cargo +1.88.0 check --workspace --all-targets
```

Every unsafe block is forbidden at the workspace lint level. Dependencies must
pass `cargo deny check` for advisories, acceptable licenses, duplicate-version
policy, and approved registries.
