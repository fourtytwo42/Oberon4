# Oberon4 Release Readiness

Version: 0.1
Status: active

## Required Gates
- [ ] `EG-P0` through `EG-P7` are green
- [ ] `EG-P8` full workspace gate is green
- [ ] `docs/plans/plan-oberon4-v0.1-build.md` has all tasks checked

## Verification Evidence
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo test -p oberon4-cli --test end_to_end`

## Requirement Readiness
- [ ] `FR-001` daemon runtime verified
- [ ] `FR-002` TUI command and chat flow verified
- [ ] `FR-003` OpenAI-compatible endpoint path verified
- [ ] `FR-004` modular crate boundaries preserved
- [ ] `FR-005` tray contract and fallback verified
- [ ] `FR-006` tests present for each subsystem
- [ ] `FR-007` OpenAI and Anthropic OAuth flows verified
- [ ] `FR-008` CLI surface verified
- [ ] `FR-009` TUI surface verified

## Human Test Entry
- [ ] `docs/runbooks/oberon4-human-test-checklist.md` exists
- [ ] human smoke pass completed
- [ ] no open stop-ship defects remain
