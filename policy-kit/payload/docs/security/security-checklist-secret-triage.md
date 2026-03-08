# Security Checklist: Secret Triage

Version: 0.1
Status: active

## Threat Model
- Framework: `STRIDE`
- Primary concern: committed credentials or credential-like material in repository history or tracked content.

## Controls Verified
- [ ] `bash scripts/bootstrap-security-tools.sh`
- [ ] `bash scripts/oberon4-security-gate.sh`
- [ ] `security/gitleaks.toml` is present and reviewed
- [ ] `security/gitleaks-baseline.json` exists and contains only approved quarantined findings
- [ ] new findings outside the baseline fail the gate
- [ ] no inline ignore comments were added to suppress findings

## Data Handling
- [ ] reports are stored under `artifacts/security-gate/`
- [ ] report sharing uses redacted outputs
- [ ] no new secret values were introduced in docs, tests, or baseline refresh notes

## Residual Risk
- Baseline entries still represent repo debt until rotation, removal, or history rewrite is completed.
