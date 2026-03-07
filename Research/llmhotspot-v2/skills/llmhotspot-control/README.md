# LLM Hotspot OpenClaw Skill

This skill package gives OpenClaw agents API-based control over LLM Hotspot.

## Files

- `SKILL.md`: OpenClaw skill definition and usage instructions.
- `llmhotspot_admin.py`: helper client for local and remote admin calls.

## Install

Place this directory into one of:

- `<workspace>/skills/llmhotspot-control`
- `~/.openclaw/skills/llmhotspot-control`

Then restart OpenClaw or refresh skills.

## Target Examples

- Local API base: `http://192.168.50.239:51824/v1`
- Remote console URL: `https://example.llmhotspot.com/remote?admin_key=adm_...`

The script auto-converts remote console URL to API base and extracts `admin_key`.

## Example Commands

```bash
python llmhotspot_admin.py discover --target "https://example.llmhotspot.com/remote?admin_key=adm_..."
python llmhotspot_admin.py request --target "http://192.168.50.239:51824/v1" --method GET --path /v1/keys
python llmhotspot_admin.py request --target "https://example.llmhotspot.com/remote?admin_key=adm_..." --method POST --path /v1/oauth/accounts/<account_id>/enable
```