$ErrorActionPreference = 'Stop'

param(
  [Parameter(Mandatory = $true)]
  [ValidateSet('orchestrator','implementer','verifier','reviewer')]
  [string]$Role,

  [Parameter(Mandatory = $true)]
  [string]$TaskId,

  [string]$Cd = ".",
  [string]$Prompt = "",
  [string]$PromptFile = "",
  [switch]$ForceUnlock,
  [int]$LockTimeoutSecs = 1800
)

function FailWithIncident {
  param(
    [string]$Category,
    [string]$Summary,
    [string]$Details
  )

  $incidentDir = Join-Path $Cd "docs/incidents"
  New-Item -ItemType Directory -Force -Path $incidentDir | Out-Null

  $slug = (Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ")
  $ts = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
  $path = Join-Path $incidentDir "agent-incident-$TaskId-$slug.md"

  @"
# Agent Incident: $Category

## Summary
$Summary

## Incident Metadata
- Timestamp: $ts
- Task ID: $TaskId
- Role: $Role
- AgentDepth: $agentDepth
- ParentRole: $parentRole
- ParentSession: $parentSession

## Trigger Details
$Details

## Immediate Mitigation
- Stop affected session.
- Return control to orchestrator.
- Preserve evidence and manifests.

## Follow-Up
- Root cause:
- Rule gap:
- Validator/hook update:
- Verification evidence:
"@ | Set-Content -Path $path -Encoding UTF8

  Write-Error "AgentIncident: $path"
  exit 1
}

if (-not (Test-Path $Cd -PathType Container)) {
  throw "Working directory not found: $Cd"
}

if ([string]::IsNullOrWhiteSpace($Prompt) -and [string]::IsNullOrWhiteSpace($PromptFile)) {
  throw "Provide -Prompt or -PromptFile."
}
if (-not [string]::IsNullOrWhiteSpace($Prompt) -and -not [string]::IsNullOrWhiteSpace($PromptFile)) {
  throw "Provide only one of -Prompt or -PromptFile."
}
if ($PromptFile -and -not (Test-Path $PromptFile -PathType Leaf)) {
  throw "Prompt file not found: $PromptFile"
}

$agentDepth = if ($env:AGENT_DEPTH) { [int]$env:AGENT_DEPTH } else { 0 }
$agentCanSpawn = if ($env:AGENT_CAN_SPAWN) { [int]$env:AGENT_CAN_SPAWN } else { 0 }
$spawnBudget = if ($env:AGENT_SPAWN_BUDGET) { [int]$env:AGENT_SPAWN_BUDGET } else { 3 }
$parentRole = if ($env:AGENT_PARENT_ROLE) { $env:AGENT_PARENT_ROLE } else { "none" }
$parentSession = if ($env:AGENT_PARENT_SESSION) { $env:AGENT_PARENT_SESSION } else { "none" }

if ($agentDepth -gt 1) {
  FailWithIncident -Category "loop-violation" -Summary "Agent depth exceeded hard limit." -Details "AGENT_DEPTH=$agentDepth"
}
if ($Role -ne "orchestrator" -and $agentCanSpawn -eq 1) {
  FailWithIncident -Category "delegation-violation" -Summary "Subagent attempted to enable spawning." -Details "role=$Role, AGENT_CAN_SPAWN=$agentCanSpawn"
}
if ($Role -eq "orchestrator" -and $agentDepth -ne 0) {
  FailWithIncident -Category "delegation-violation" -Summary "Orchestrator started with invalid depth." -Details "role=$Role, AGENT_DEPTH=$agentDepth"
}
if ($Role -ne "orchestrator") {
  $agentCanSpawn = 0
}

$stateDir = Join-Path $Cd ".agent-runtime/$TaskId"
$runsDir = Join-Path $stateDir "runs"
New-Item -ItemType Directory -Force -Path $runsDir | Out-Null

$lockPath = Join-Path $stateDir "orchestrator.lock"
$spawnCountPath = Join-Path $stateDir "spawn.count"
$lineagePath = Join-Path $stateDir "lineage.log"
$nowEpoch = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()

if (-not (Test-Path $spawnCountPath -PathType Leaf)) { "0" | Set-Content -Path $spawnCountPath -Encoding UTF8 }
$currentSpawns = [int](Get-Content $spawnCountPath -Raw).Trim()

if ($Role -eq "orchestrator") {
  if (Test-Path $lockPath -PathType Leaf) {
    $lockContent = Get-Content $lockPath -Raw
    $lockPid = ""
    $lockEpoch = 0
    foreach ($line in ($lockContent -split "`n")) {
      if ($line -match '^pid=(.+)$') { $lockPid = $Matches[1].Trim() }
      if ($line -match '^ts_epoch=(\d+)$') { $lockEpoch = [int64]$Matches[1] }
    }
    $lockAge = if ($lockEpoch -gt 0) { $nowEpoch - $lockEpoch } else { 0 }

    if ($ForceUnlock) {
      Remove-Item -Force $lockPath
    } elseif ($lockPid -and (Get-Process -Id $lockPid -ErrorAction SilentlyContinue)) {
      FailWithIncident -Category "loop-violation" -Summary "Concurrent orchestrator lock detected." -Details "task=$TaskId, pid=$lockPid, age_secs=$lockAge"
    } elseif ($lockAge -ge $LockTimeoutSecs) {
      Remove-Item -Force $lockPath
    } else {
      Remove-Item -Force $lockPath
    }
  }

  @"
pid=$PID
ts=$((Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ"))
ts_epoch=$nowEpoch
"@ | Set-Content -Path $lockPath -Encoding UTF8

  if ($agentCanSpawn -eq 1) {
    if ($currentSpawns -ge $spawnBudget) {
      FailWithIncident -Category "loop-violation" -Summary "Spawn budget exceeded." -Details "task=$TaskId, spawns=$currentSpawns, budget=$spawnBudget"
    }
    ($currentSpawns + 1).ToString() | Set-Content -Path $spawnCountPath -Encoding UTF8
  }
}

$runId = "{0}-{1}-{2}" -f ((Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ")), $Role, $PID
$manifestPath = Join-Path $runsDir "$runId.json"
$roleHeader = "Role=$Role; Task=$TaskId; Depth=$agentDepth; CanSpawn=$agentCanSpawn"
$fullPrompt = if ($PromptFile) { $roleHeader + "`n`n" + (Get-Content $PromptFile -Raw) } else { $roleHeader + "`n`n" + $Prompt }

$lineageTs = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
"$lineageTs role=$Role depth=$agentDepth can_spawn=$agentCanSpawn" | Add-Content -Path $lineagePath -Encoding UTF8

$args = @("-a","never","exec","--ephemeral","--sandbox","workspace-write","--cd",$Cd,"--skip-git-repo-check")
if ($env:AGENT_MODEL) { $args += @("--model", $env:AGENT_MODEL) }
$args += $fullPrompt

$status = "success"
$failureReason = ""
try {
  & codex @args
  if ($LASTEXITCODE -ne 0) {
    $status = "failed"
    $failureReason = "codex_exec_nonzero"
  }
} catch {
  $status = "failed"
  $failureReason = "codex_exec_exception"
}

$manifest = [ordered]@{
  run_id = $runId
  timestamp = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
  task_id = $TaskId
  role = $Role
  depth = $agentDepth
  can_spawn = $agentCanSpawn
  spawn_budget = $spawnBudget
  parent_role = $parentRole
  parent_session = $parentSession
  workdir = $Cd
  prompt_source = $(if ($PromptFile) { $PromptFile } else { "inline" })
  status = $status
  failure_reason = $failureReason
}
$manifest | ConvertTo-Json | Set-Content -Path $manifestPath -Encoding UTF8

if ($status -ne "success") {
  Write-Error "Agent run failed. Manifest: $manifestPath"
  exit 1
}
