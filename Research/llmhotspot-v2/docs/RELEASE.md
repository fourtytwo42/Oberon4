# Releases (Manual, No CI)

This repo supports building and packaging release artifacts without GitHub Actions.
You run the build scripts locally on each OS.

## Why Per-OS Machines

- Linux can build Linux artifacts on a Linux machine/VM.
- Windows installers require Windows tooling (and usually code signing for a clean UX).
- macOS `.app`/DMG and auto-update distribution requires macOS tooling (and usually codesign/notarization).

## Output Location

All scripts write artifacts to `dist/` with versioned filenames.

## Linux (Binary + Bundled cloudflared)

From the repo root:

```bash
bash scripts/release/release-linux.sh
```

Produces:
- `dist/llmhotspot-v<version>-linux-x86_64.tar.gz`

## Windows (Binary + Bundled cloudflared + NSIS Installer)

Run on a Windows machine with Rust installed:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\\release\\release-windows.ps1
```

Produces:
- `dist\\llmhotspot-v<version>-windows-x86_64.zip`
- `dist\\LLMHotspot-Setup-v<version>-windows-x86_64.exe` (when `makensis` is installed)

Installer behavior:
- Installs to `C:\Program Files\LLM Hotspot`
- Registers in Add/Remove Programs
- Overwrites same-version installs
- Blocks downgrade installs if a newer version is already installed

Requirements for Setup `.exe`:
- Install NSIS and ensure `makensis` is on `PATH`
- Installer script: `installer/windows/llmhotspot.nsi`

## macOS (Binary + Bundled cloudflared)

Run on a macOS machine with Rust installed:

```bash
bash scripts/release/release-macos.sh
```

Produces:
- `dist/llmhotspot-v<version>-macos-<arch>.tar.gz` (where `<arch>` is `x86_64` or `arm64`)

## macOS (App Bundle + DMG)

Run on a macOS machine with Rust installed:

```bash
bash scripts/release/release-macos-app.sh
```

Produces:
- `dist/LLM Hotspot.app` (contains `llmhotspot` + `cloudflared`)
- `dist/LLMHotspot-v<version>-macos-<arch>.dmg` (when `hdiutil` is available)

## Notes On Auto-Updates

If you want "updates automatically when there is a newer version":

- Windows: typically requires a signed installer and an update feed.
- macOS: typically requires codesign + notarization + an update feed.

We can implement a self-updater in the app, but the cleanest user experience depends on signing.
