# Contributing

## Rules

1. Keep crate boundaries clean; no circular dependencies.
2. Add tests for all public behavior.
3. No breaking `/v1/*` contract changes without versioning notes.
4. Keep routes thin in `llm-control-api`; domain logic belongs in domain crates.

## Required checks

Run `make ci` before pushing.

## Windows Test Troubleshooting

When `cargo test` fails with linker error `LNK1104` for a test executable (for example
`codex_tool_call_flow_tests-*.exe`), a stale process is usually holding the binary open.

Use this cleanup command, then rerun tests:

```powershell
Get-Process |
  Where-Object {
    $_.ProcessName -like 'codex_tool_call_flow_tests*' -or
    $_.ProcessName -like 'cargo' -or
    $_.ProcessName -like 'rustc' -or
    $_.ProcessName -like 'link'
  } |
  Stop-Process -Force
```

For debugging flaky integration tests, run a targeted suite single-threaded:

```powershell
cargo test -p llm-control-api --test codex_tool_call_flow_tests -- --test-threads=1
```
