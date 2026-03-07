# Platform Smoke Checklist

## Tray Behavior

### Linux
- Start: `llmhotspot serve` (without `--no-tray`).
- Verify left click opens `/console` URL.
- Verify right-click menu has `Open Console`, `Restart App`, `Exit`.
- Verify `Restart App` sets restart marker and app remains responsive.
- Verify `Exit` stops listener cleanly.

### macOS
- Start native tray build.
- Verify icon appears in menu bar.
- Verify left click opens console URL.
- Verify menu actions map to open/restart/exit.

### Windows
- Start native tray build.
- Verify system tray icon visible.
- Verify context menu actions map to open/restart/exit.

## Local/Remote UI parity
- Open local `/console` and remote console URL.
- Compare cards and actions:
  - Header status
  - Endpoints with copy buttons
  - OAuth account actions
  - API key create/copy/revoke
  - Diagnostics counters

## Mobile responsiveness
- Test viewport widths: 390, 430, 768.
- Verify no horizontal overflow in OAuth/API keys tables.
- Verify all action buttons reachable and keyboard focusable.

## Security/Auth checks
- With no keys: privileged routes require `Bearer $LLMHOTSPOT_BOOTSTRAP_TOKEN`.
- After first key creation: bootstrap token denied, API key required.
- Revoked/expired key denied on privileged routes and chat forwarding.

## Control-plane lease behavior
- Verify assignment fetched from `/assignment` endpoint.
- Verify lease expiry timestamp stored/exposed.
- Simulate refresh failure after lease expiry and verify stale state is shown with last error.
