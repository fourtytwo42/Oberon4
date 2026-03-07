$ErrorActionPreference = 'Stop'

$bash = Get-Command bash -ErrorAction SilentlyContinue
if ($bash) {
  & bash scripts/validate-push.sh
  exit $LASTEXITCODE
}

Write-Error "bash is required to run push validation in this repo. Install Git Bash or WSL."
exit 1
