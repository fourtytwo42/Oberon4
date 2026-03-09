# Runbook: WASM Plugin Runtime WP2 Stabilization

## Purpose
- verify the Wasmtime dependency remediation and the reproducible local verification profile for the `WP2` execution layer

## Verification
- run `cargo test -p oberon4-daemon`
- run `cargo test -p oberon4-daemon --test plugin_runtime_execution`
- run `cargo test --workspace`
- run `cargo clippy --workspace --all-targets -- -D warnings`
- run `PATH="$HOME/.cargo/bin:$PATH" bash scripts/oberon4-security-gate.sh`
- run `GATE_MODE=strict RUN_SECURITY_GATE=1 bash scripts/oberon4-gate.sh`
- run `bash scripts/local-ci-gate.sh`

Expected outcomes:
- Wasmtime-backed plugin execution still passes the `WP2` runtime tests
- `cargo audit` inside the security gate no longer reports the Wasmtime advisories
- the strict and local CI gates pass with the constrained Rust build profile

## Constrained Local Rust Profile
The canonical gate now defaults to:
- `TMPDIR=/home/hendo420/Oberon4/.tmp`
- `CARGO_TARGET_DIR=/home/hendo420/Oberon4/target`
- `CARGO_INCREMENTAL=0`
- `CARGO_PROFILE_DEV_DEBUG=0`
- `CARGO_PROFILE_TEST_DEBUG=0`
- `CARGO_BUILD_JOBS=1`
- `RUSTFLAGS='-C debuginfo=0'`

Safe override path:
- export different values before invoking `bash scripts/oberon4-gate.sh` or `bash scripts/local-ci-gate.sh` when a machine needs a different local build profile

## Rollback
1. revert the stabilization patch commit
2. remove `rust-toolchain.toml` if reverting the toolchain pin
3. restore the previous daemon Wasmtime dependency line if necessary
4. rerun:
   - `cargo test -p oberon4-daemon`
   - `cargo test -p oberon4-daemon --test plugin_runtime_execution`

## Operational Note
- if local linker pressure still appears during verification on this host, confirm active swap with `swapon --show` before changing repo configuration further
