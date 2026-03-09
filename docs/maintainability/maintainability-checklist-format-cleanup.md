# Maintainability Checklist: Format Cleanup

- Scope is limited to formatting-only edits in:
  - `crates/oberon4-auth/src/lib.rs`
  - `crates/oberon4-cli/tests/daemon_start.rs`
  - `crates/oberon4-tui/src/lib.rs`
- No control flow, data flow, or public interface behavior is changed.
- Formatting remains consistent with `cargo fmt --check --all`.
- Targeted crate tests cover the touched areas after formatting.
- No dependency, schema, policy, or runtime behavior changes are included.
