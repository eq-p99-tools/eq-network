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

The automatic [PR checks](.github/workflows/lint.yml) verify formatting and
dependency policy on Windows. To run the full checks locally on Windows with
the default CI toolchain, use PowerShell.

```powershell
cargo +1.99.0 fmt --all -- --check
cargo +1.99.0 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.99.0 test --workspace --all-targets --all-features
$env:RUSTDOCFLAGS = '-D warnings'
cargo +1.99.0 doc --workspace --all-features --no-deps
```

The manual [CI workflow](.github/workflows/ci.yml) runs Clippy, tests and
rustdoc on one selected platform. Windows and Rust 1.99.0 are the defaults.
Select Linux explicitly for a Linux check.

Record a change's changelog entry as a new file in `changelog.d/`, not as an
edit to `CHANGELOG.md`, so that pull requests never conflict over the
changelog; [changelog.d/README.md](changelog.d/README.md) has the details.
Maintainers fold the fragments in with `python3 contrib/fold-changelog.py`.

Every unsafe block is forbidden at the workspace lint level. Dependencies must
pass `cargo deny check` for advisories, acceptable licenses, duplicate-version
policy, and approved registries.
