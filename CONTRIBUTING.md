# Contributing

Use Rust 2021 and the declared MSRV (1.89). Windows 10/11 is the product target;
Linux builds support library and headless GUI development. Install the platform
build dependencies required by eframe and, for packaging, Inno Setup 6.7.1.

```sh
cargo build --locked
cargo fmt --all --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo clippy --target x86_64-pc-windows-msvc --all-targets --all-features --locked -- -D warnings
cargo test --locked
cargo check --target x86_64-pc-windows-msvc --all-targets --all-features --locked
cargo audit
cargo deny check
```

Run the Windows-target commands with the MSVC toolchain on Windows (the
signature verifier builds native code). Install cargo-audit/cargo-deny separately. Normal tests
must not contact external services; the dedicated hosted installer/media field
jobs have explicit network access and are not normal unit tests. Keep `Cargo.lock`
committed. For each PR describe user-visible behavior, tests, security impact,
Windows/physical-DPI limitations and any new dependencies. Do not weaken process
containment, URL validation, clipboard limits or checksum verification. Follow
[AGENTS.md](AGENTS.md) and [design verification](docs/DESIGN.md).

## Fuzzing

With Rust nightly and `cargo-fuzz` installed, run a bounded local smoke test:

```sh
cargo +nightly fuzz run url_input -- -max_total_time=30
cargo +nightly fuzz run clipboard_input -- -max_total_time=30
cargo +nightly fuzz run release_signature -- -max_total_time=30
```

Fuzzing is separate from ordinary PR tests and runs on a weekly schedule. Never
check in generated crash artifacts containing private clipboard data. Add small,
synthetic reproductions to the seed corpus after reviewing them.

For vulnerabilities, follow [SECURITY.md](SECURITY.md), not a public issue.
