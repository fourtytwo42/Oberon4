#!/usr/bin/env node
/* eslint-disable no-console */
const fs = require('fs');
const path = require('path');
const { spawn, spawnSync } = require('child_process');

const ROOT = process.cwd();
const DEFAULT_STATE = path.join(ROOT, '.agent-orchestration', 'state.json');
const RUN_AGENT_SH = path.join(ROOT, 'scripts', 'run-agent.sh');
const CONTROL_JS = path.join(ROOT, 'scripts', 'agent-control.js');
const GIT_SYNC_SH = path.join(ROOT, 'scripts', 'agent-git-sync.sh');
const PROMPT_DIR = path.join(ROOT, '.agent-runtime', 'autopilot', 'prompts');
const AUTOPILOT_EVENT_LOG = path.join(ROOT, '.agent-runtime', 'autopilot', 'events.log');
const PHASES = ['build', 'reconcile', 'verify', 'fix'];
const REPO_REALITY_HINTS = [
  'Important repo reality: planned `crates/core-*` modules are not all present in this checkout.',
  'Use existing modules in this repository; do not spend cycles searching for missing `core-*` crate paths.',
];

function parseArgs(argv) {
  const out = { _: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const t = argv[i];
    if (!t.startsWith('--')) {
      out._.push(t);
      continue;
    }
    const key = t.slice(2);
    const next = argv[i + 1];
    if (!next || next.startsWith('--')) {
      out[key] = true;
      continue;
    }
    out[key] = next;
    i += 1;
  }
  return out;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function autopilotLog(message) {
  try {
    fs.mkdirSync(path.dirname(AUTOPILOT_EVENT_LOG), { recursive: true });
    fs.appendFileSync(AUTOPILOT_EVENT_LOG, `${new Date().toISOString()} ${message}\n`);
  } catch (_) {
    // best-effort only
  }
}

function nowSlug() {
  return new Date().toISOString().replace(/[:.]/g, '-');
}

function loadState(stateFile) {
  return JSON.parse(fs.readFileSync(stateFile, 'utf8'));
}

async function loadStateWithRetry(stateFile, attempts = 5, delayMs = 120) {
  let lastErr = null;
  for (let i = 0; i < attempts; i += 1) {
    try {
      return loadState(stateFile);
    } catch (err) {
      lastErr = err;
      // Retry parse/read races while state file is being replaced.
      const isFinal = i === attempts - 1;
      if (!isFinal) await sleep(delayMs);
    }
  }
  throw lastErr;
}

function getCurrentBatch(state) {
  if (!state.pipeline || !Array.isArray(state.pipeline.batchOrder) || state.pipeline.batchOrder.length === 0) return null;
  const idx = state.pipeline.currentBatchIndex || 0;
  return state.pipeline.batchOrder[idx] || null;
}

function jobInCurrentBatch(state, job) {
  const current = getCurrentBatch(state);
  if (current == null) return true;
  return job.batch === current;
}

function jobReadyForPhase(job, phase) {
  const s = job.stage || {};
  if (phase === 'build') return !s.buildDone;
  if (phase === 'reconcile') return s.buildDone && !s.reconcileDone;
  if (phase === 'verify') return s.reconcileDone && s.verifyResult === 'pending';
  if (phase === 'fix') return s.verifyResult === 'fail' && !s.fixDone;
  return false;
}

function listReadyJobs(state, phase) {
  return Object.values(state.jobs).filter((job) => jobInCurrentBatch(state, job) && jobReadyForPhase(job, phase));
}

function selectDispatchJobs(readyJobs, maxJobs, maxPerDepartment = 1) {
  if (!Array.isArray(readyJobs) || readyJobs.length === 0) return [];
  const limit = Math.max(1, Number(maxJobs || 1));
  const perDeptLimit = Math.max(1, Number(maxPerDepartment || 1));
  const departmentCounts = new Map();
  const selected = [];

  for (const job of readyJobs) {
    const dept = job.department || `__no_department__:${job.id}`;
    const current = departmentCounts.get(dept) || 0;
    if (current >= perDeptLimit) continue;
    selected.push(job);
    departmentCounts.set(dept, current + 1);
    if (selected.length >= limit) break;
  }

  // Safety fallback: always dispatch at least one job if anything is ready.
  if (selected.length === 0) selected.push(readyJobs[0]);
  return selected;
}

function runControl(stateFile, args, allowFail = false) {
  const fullArgs = [CONTROL_JS, ...args, '--state-file', stateFile];
  const res = spawnSync('node', fullArgs, {
    cwd: ROOT,
    encoding: 'utf8',
  });
  if (res.stdout) process.stdout.write(res.stdout);
  if (res.stderr) process.stderr.write(res.stderr);
  const ok = res.status === 0;
  if (!ok && !allowFail) {
    throw new Error(`agent-control failed: node ${fullArgs.join(' ')}`);
  }
  return { ok, status: res.status, stdout: res.stdout || '', stderr: res.stderr || '' };
}

function roleForPhase(phase) {
  if (phase === 'build') return 'implementer';
  if (phase === 'reconcile') return 'reviewer';
  if (phase === 'verify') return 'verifier';
  if (phase === 'fix') return 'implementer';
  return 'implementer';
}

function noteForSuccess(phase, job) {
  if (phase === 'build') return `autopilot build done (${job.endGate})`;
  if (phase === 'reconcile') return `autopilot reconcile done (${job.endGate})`;
  if (phase === 'verify') return `autopilot verify pass (${job.endGate})`;
  if (phase === 'fix') return `autopilot fix done (${job.endGate})`;
  return `autopilot done (${job.endGate})`;
}

function buildPrompt(job, phase) {
  const contextHints = moduleHintsForJob(job);
  const header = [
    `Micro-job: ${job.id} (${job.endGate})`,
    `Scope: ${job.scope}`,
    `Phase: ${phase}`,
    '',
    'Context links only (do not paste large file contents):',
    '- docs/plans/plan-prd-oberon.md',
    '- docs/plans/plan-agent-work-packages.md',
    '- docs/plans/plan-agent-micro-jobs.md',
    ...contextHints.map((v) => `- ${v}`),
    '',
    'Token discipline (hard):',
    '- Read only targeted sections/line ranges.',
    '- Do not scan whole docs; use rg/nl first.',
    '- Keep edits scoped to this micro-job only.',
    '- Run only targeted tests for this scope.',
    '- End response with short evidence: changed files + test command + result.',
    '',
  ];

  if (phase === 'build') {
    return `${header.join('\n')}Implement the micro-job and run targeted tests. Exit non-zero on failure.`;
  }
  if (phase === 'reconcile') {
    return `${header.join('\n')}Reconcile implementation against end gate and PRD requirements. Apply minimal corrective edits, run targeted tests, exit non-zero on unresolved gaps.`;
  }
  if (phase === 'verify') {
    return `${header.join('\n')}Verify this micro-job with targeted tests only, confirm end gate, report pass/fail evidence. Exit non-zero if verification fails.`;
  }
  return `${header.join('\n')}Fix verification failures, run targeted tests, confirm end-gate compliance. Exit non-zero if still failing.`;
}

function moduleHintsForJob(job) {
  const hints = [];
  if (job.department === 'runtime-core') {
    hints.push('crates/llmhotspot/src/runtime_ipc.rs');
    hints.push('crates/llmhotspot/src/main.rs');
  }
  if (job.department === 'policy-security') {
    hints.push('crates/llm-governance/src/policy.rs');
    hints.push('crates/llm-governance/tests/policy_tests.rs');
    hints.push('contracts/policy/*.schema.json');
    hints.push('docs/specs/policy-engine-v1.md');
  }
  if (job.department === 'endpoint-runner') {
    hints.push('crates/llmhotspot/src/tunnel_broker.rs');
    hints.push('crates/llmhotspot/src/tunnel_broker/cloudflare.rs');
  }
  if (job.department === 'ui-integration') {
    hints.push('apps/website/**');
    hints.push('crates/llm-tray/**');
  }
  if (job.department === 'messaging') {
    hints.push('crates/llm-control-api/**');
    hints.push('crates/llmhotspot/**');
  }
  if (job.department === 'packaging-distribution') {
    hints.push('installer/**');
    hints.push('scripts/**');
    hints.push('crates/llm-tray/**');
  }

  if (job.id === 'M04' || job.id === 'M05') {
    hints.push('M04/M05: use existing runtime module, no new crates');
  }
  if (job.id === 'M06' || job.id === 'M07' || job.id === 'M08' || job.id === 'M09') {
    hints.push('M06-M09: llm-governance is the policy engine locus');
  }
  if (job.id === 'M10' || job.id === 'M11' || job.id === 'M12' || job.id === 'M13') {
    hints.push('M10-M13: keep changes minimal in governance/storage surfaces');
  }

  return hints;
}

function writePrompt(job, phase) {
  fs.mkdirSync(PROMPT_DIR, { recursive: true });
  const promptPath = path.join(PROMPT_DIR, `${job.id}-${phase}-${nowSlug()}.txt`);
  fs.writeFileSync(promptPath, buildPrompt(job, phase));
  return promptPath;
}

function runWorker(job, phase, stateFile) {
  return new Promise((resolve) => {
    const role = roleForPhase(phase);
    const agentName = `${role}-${job.id.toLowerCase()}`;
    const promptFile = writePrompt(job, phase);

    try {
      runControl(stateFile, ['start', job.id, '--agent', agentName]);
    } catch (err) {
      resolve({ ok: false, job, phase, reason: `start_failed: ${err.message}` });
      return;
    }

    console.log(`[autopilot] ${phase} dispatch ${job.id} via ${agentName}`);
    const child = spawn('bash', [
      RUN_AGENT_SH,
      '--role', role,
      '--task-id', job.id,
      '--cd', ROOT,
      '--prompt-file', promptFile,
    ], {
      cwd: ROOT,
      stdio: 'inherit',
    });

    child.on('exit', (code, signal) => {
      const success = code === 0 && !signal;
      try {
        if (success) {
          if (phase === 'build') {
            runControl(stateFile, ['complete', job.id, '--note', noteForSuccess(phase, job)]);
          } else if (phase === 'reconcile') {
            runControl(stateFile, ['reconcile-done', job.id, '--note', noteForSuccess(phase, job)]);
          } else if (phase === 'verify') {
            runControl(stateFile, ['verify', job.id, '--result', 'pass', '--note', noteForSuccess(phase, job)]);
          } else if (phase === 'fix') {
            runControl(stateFile, ['fix-done', job.id, '--note', noteForSuccess(phase, job)]);
          }
          resolve({ ok: true, job, phase });
          return;
        }

        const failureReason = `autopilot ${phase} worker failed (code=${code ?? 'n/a'} signal=${signal ?? 'none'})`;
        runControl(
          stateFile,
          [
            'raise',
            job.id,
            '--agent',
            agentName,
            '--severity',
            phase === 'verify' ? 'high' : 'medium',
            '--text',
            failureReason,
          ],
          true,
        );

        if (phase === 'verify') {
          runControl(
            stateFile,
            ['verify', job.id, '--result', 'fail', '--note', 'autopilot verify failed (worker exited non-zero)'],
            true,
          );
          resolve({ ok: false, job, phase, reason: 'verify_failed' });
          return;
        }

        runControl(
          stateFile,
          ['fail', job.id, '--reason', failureReason],
          true,
        );
        resolve({ ok: false, job, phase, reason: `${phase}_failed` });
      } catch (err) {
        resolve({ ok: false, job, phase, reason: `state_update_failed: ${err.message}` });
      }
    });
  });
}

async function runWithConcurrency(items, limit, workerFn) {
  if (items.length === 0) return [];
  const max = Math.max(1, limit);
  const results = new Array(items.length);
  let nextIndex = 0;
  let active = 0;

  return new Promise((resolve) => {
    const launch = () => {
      while (active < max && nextIndex < items.length) {
        const idx = nextIndex;
        nextIndex += 1;
        active += 1;
        workerFn(items[idx], idx)
          .then((res) => {
            results[idx] = res;
          })
          .catch((err) => {
            results[idx] = { ok: false, error: err.message || String(err) };
          })
          .finally(() => {
            active -= 1;
            if (nextIndex >= items.length && active === 0) {
              resolve(results);
            } else {
              launch();
            }
          });
      }
    };
    launch();
  });
}

function summarizeResults(results) {
  let ok = 0;
  let failed = 0;
  for (const r of results) {
    if (r && r.ok) ok += 1;
    else failed += 1;
  }
  return { ok, failed };
}

function maybeGitSync(stateFile, reason, enabled) {
  if (!enabled) return;
  if (!fs.existsSync(GIT_SYNC_SH)) {
    autopilotLog(`warn git_sync_missing script=${GIT_SYNC_SH}`);
    return;
  }
  const res = spawnSync('bash', [GIT_SYNC_SH, '--state-file', stateFile, '--reason', reason], {
    cwd: ROOT,
    encoding: 'utf8',
  });
  if (res.stdout) process.stdout.write(res.stdout);
  if (res.stderr) process.stderr.write(res.stderr);
  if (res.status !== 0) {
    autopilotLog(`warn git_sync_failed code=${res.status} reason=${reason}`);
  } else {
    autopilotLog(`git_sync_ok reason=${reason}`);
  }
}

function activeInProgressCount(state) {
  return Object.values(state.jobs || {}).filter((j) => j.status === 'in_progress').length;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const stateFile = path.resolve(String(args['state-file'] || DEFAULT_STATE));
  const concurrency = Math.max(1, Number(args.concurrency || 4));
  const maxPerDepartment = Math.max(1, Number(args['max-per-dept'] || 1));
  const loop = Boolean(args.loop);
  const idleOnComplete =
    Number(args['idle-on-complete'] ?? (process.env.pm_id !== undefined ? 1 : 0)) !== 0;
  const autoGitSync = String(args['auto-git-sync'] ?? '1') !== '0';
  const sleepSeconds = Math.max(1, Number(args['sleep-sec'] || 3));
  const maxCycles = idleOnComplete
    ? Number.POSITIVE_INFINITY
    : Math.max(1, Number(args['max-cycles'] || (loop ? 100000 : 1)));

  if (!fs.existsSync(stateFile)) {
    console.error(`State file not found: ${stateFile}`);
    process.exit(1);
  }
  if (!fs.existsSync(RUN_AGENT_SH)) {
    console.error(`Missing runner script: ${RUN_AGENT_SH}`);
    process.exit(1);
  }

  console.log(`[autopilot] state=${stateFile} concurrency=${concurrency} loop=${loop} autoGitSync=${autoGitSync}`);
  autopilotLog(`start state=${stateFile} concurrency=${concurrency} loop=${loop} autoGitSync=${autoGitSync}`);

  let cycles = 0;
  while (cycles < maxCycles) {
    cycles += 1;
    const state = await loadStateWithRetry(stateFile);
    const phase = state.pipeline?.phase || 'build';
    const runControlState = state.pipeline?.runControl || { mode: 'running', pauseMode: null };
    if (!PHASES.includes(phase)) {
      console.error(`[autopilot] unknown phase: ${phase}`);
      autopilotLog(`fatal unknown_phase=${phase}`);
      process.exit(1);
    }
    if (state.pipeline?.completed) {
      console.log('[autopilot] pipeline already completed; idling.');
      autopilotLog('tick completed=true idle');
      if (!loop && !idleOnComplete) break;
      await sleep(sleepSeconds * 1000);
      continue;
    }

    if (runControlState.mode === 'paused') {
      console.log('[autopilot] paused; dispatch is disabled.');
      autopilotLog('tick paused');
      if (!loop) break;
      await sleep(sleepSeconds * 1000);
      continue;
    }

    if (runControlState.mode === 'pause_requested') {
      const active = activeInProgressCount(state);
      if (active === 0) {
        const finalized = runControl(stateFile, ['pause-finalize'], true);
        if (finalized.ok) {
          console.log('[autopilot] drain pause finalized.');
          autopilotLog('tick drain_pause_finalized');
        } else {
          console.log('[autopilot] pause-finalize failed; retrying.');
          autopilotLog('warn pause_finalize_failed');
        }
      } else {
        console.log(`[autopilot] drain pause active; waiting for ${active} in-progress job(s) to finish.`);
        autopilotLog(`tick drain_wait active=${active}`);
      }
      if (!loop) break;
      await sleep(sleepSeconds * 1000);
      continue;
    }

    const batch = getCurrentBatch(state);
    const ready = listReadyJobs(state, phase);
    console.log(`[autopilot] cycle=${cycles} batch=${batch ?? 'n/a'} phase=${phase} ready=${ready.length}`);
    autopilotLog(`tick cycle=${cycles} batch=${batch ?? 'n/a'} phase=${phase} ready=${ready.length}`);

    if (ready.length === 0) {
      const adv = runControl(stateFile, ['advance'], true);
      if (adv.ok) {
        maybeGitSync(stateFile, `phase advance (${phase}) with empty ready queue`, autoGitSync);
        continue;
      }
      if (!loop) break;
      await sleep(sleepSeconds * 1000);
      continue;
    }

    const dispatchJobs = selectDispatchJobs(ready, concurrency, maxPerDepartment);
    console.log(
      `[autopilot] dispatching ${dispatchJobs.length}/${ready.length} ready job(s) with per-dept cap=${maxPerDepartment}`,
    );
    autopilotLog(`dispatch count=${dispatchJobs.length} ready=${ready.length} phase=${phase}`);
    // Enforce the configured global worker cap even after department-based selection.
    const results = await runWithConcurrency(dispatchJobs, concurrency, (job) => runWorker(job, phase, stateFile));
    const summary = summarizeResults(results);
    console.log(`[autopilot] phase=${phase} results ok=${summary.ok} failed=${summary.failed}`);
    autopilotLog(`results phase=${phase} ok=${summary.ok} failed=${summary.failed}`);

    const adv = runControl(stateFile, ['advance'], true);
    if (adv.ok) {
      maybeGitSync(stateFile, `phase advance (${phase}) after worker results`, autoGitSync);
    } else {
      maybeGitSync(stateFile, `post-dispatch checkpoint (${phase})`, autoGitSync);
    }

    if (!loop) break;
  }
}

main().catch((err) => {
  autopilotLog(`fatal unhandled=${err?.stack || err?.message || String(err)}`);
  console.error(err.stack || err.message || String(err));
  process.exit(1);
});

process.on('SIGTERM', () => {
  autopilotLog('signal SIGTERM');
  process.exit(143);
});

process.on('SIGINT', () => {
  autopilotLog('signal SIGINT');
  process.exit(130);
});
