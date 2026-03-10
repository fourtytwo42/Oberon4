# Task Brief: GitHub OAuth Login

## Goal
Ship GitHub OAuth login through the existing Oberon4 account pipeline so the versioned workspace package can use real brokered GitHub credentials.

## Must
- Reuse the current localhost browser callback model
- Keep tokens host-custodied
- Support CLI and TUI entrypoints
- Keep runtime GitHub operations brokered

## Should
- Keep provider routing symmetrical with OpenAI and Anthropic
- Avoid new protocol or IPC surfaces unless strictly necessary

## Acceptance
- `AC-201`: CLI GitHub connect flow works through the daemon
- `AC-202`: TUI `/github-login` works through the daemon
- `AC-203`: runtime broker can resolve the active GitHub account
- `AC-204`: no raw token exposure
