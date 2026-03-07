# Agent Policy Kit

Reusable policy stack for new repositories.

## Versioning

- Current version: see `policy-kit/VERSION`
- Change history: `policy-kit/CHANGELOG.md`
- Recommended usage:
  - pin a tagged release of `bos-policy-kit`
  - update intentionally and review changelog deltas before applying

## Install into a new/existing repo

```bash
bash policy-kit/bootstrap.sh --target /path/to/repo --repo-slug owner/repo
```

If run from inside the target repo, `--target` is optional:

```bash
bash /path/to/agent-policy-kit/policy-kit/bootstrap.sh --repo-slug owner/repo
```

## Update an existing repo from kit payload

```bash
bash policy-kit/update.sh --target /path/to/repo
```

## Refresh payload from this source repo

```bash
bash policy-kit/build-payload.sh
```

## Verify bootstrap in target repo

```bash
bash scripts/bootstrap-policy-stack.sh
```

## Notes

- `bootstrap.sh` installs files from `policy-kit/payload/`.
- Existing files are preserved by default; pass `--force` to overwrite.
- `scripts/hooks.env` is preserved by default even with `--force`.
