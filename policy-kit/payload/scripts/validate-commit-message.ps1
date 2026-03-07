param(
  [Parameter(Mandatory=$true)]
  [string]$MessageFile
)

$ErrorActionPreference = 'Stop'

$bash = Get-Command bash -ErrorAction SilentlyContinue
if ($bash) {
  & bash scripts/validate-commit-message.sh "$MessageFile"
  exit $LASTEXITCODE
}

Write-Error "bash is required to run commit-message validation in this repo. Install Git Bash or WSL."
exit 1
