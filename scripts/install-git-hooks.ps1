$ErrorActionPreference = 'Stop'

$repoRoot = Resolve-Path "$PSScriptRoot/.."
Set-Location $repoRoot

git config core.hooksPath .githooks | Out-Null

Write-Host "Installed local git hooks at .githooks"
$hooksPath = git config --get core.hooksPath
Write-Host "Current hooksPath: $hooksPath"
Write-Host "Note: Hooks run through shell scripts; use Git Bash (or WSL) for full compatibility."
