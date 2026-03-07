#!/usr/bin/env node
/* eslint-disable no-console */
const fs = require('fs');
const path = require('path');

const ROOT = process.cwd();
const DEFAULT_STATE = path.join(ROOT, '.agent-orchestration', 'state.json');
const DEFAULT_PLAN = path.join(ROOT, 'docs', 'plans', 'plan-agent-micro-jobs.md');
const PHASES = ['build', 'reconcile', 'verify', 'fix'];

function nowIso() {
  return new Date().toISOString();
}

function asMs(iso) {
  return new Date(iso).getTime();
}

function ensureDir(file) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
}

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

function loadState(file) {
  if (!fs.existsSync(file)) {
    return null;
  }
  return JSON.parse(fs.readFileSync(file, 'utf8'));
}

function saveState(file, state) {
  state.updatedAt = nowIso();
  ensureDir(file);
  // Atomic replace to avoid readers seeing partially-written JSON.
  const tmp = `${file}.tmp.${process.pid}.${Date.now()}`;
  fs.writeFileSync(tmp, JSON.stringify(state, null, 2));
  fs.renameSync(tmp, file);
}

function migrateState(state) {
  if (!state.pipeline) {
    state.pipeline = {
      phase: 'build',
      batchOrder: [1],
      currentBatchIndex: 0,
      completed: false,
    };
  }
  if (!Array.isArray(state.pipeline.batchOrder) || state.pipeline.batchOrder.length === 0) {
    state.pipeline.batchOrder = [1];
  }
  if (!state.pipeline.runControl || typeof state.pipeline.runControl !== 'object') {
    state.pipeline.runControl = {
      mode: 'running', // running|pause_requested|paused
      pauseMode: null, // drain|immediate|null
      reason: null,
      requestedAt: null,
      pausedAt: null,
    };
  }
  if (!Array.isArray(state.events)) state.events = [];
  if (!Array.isArray(state.concerns)) state.concerns = [];
  if (!Number.isFinite(state.concernSeq)) state.concernSeq = 0;
  for (const job of Object.values(state.jobs || {})) {
    if (!job.stage) {
      job.stage = {
        buildDone: false,
        reconcileDone: false,
        verifyResult: 'pending',
        fixDone: false,
      };
    } else {
      job.stage.buildDone = Boolean(job.stage.buildDone);
      job.stage.reconcileDone = Boolean(job.stage.reconcileDone);
      if (!['pending', 'pass', 'fail'].includes(job.stage.verifyResult)) {
        job.stage.verifyResult = 'pending';
      }
      job.stage.fixDone = Boolean(job.stage.fixDone);
    }
    if (job.batch == null) {
      job.batch = state.pipeline.batchOrder[0];
    }
  }
  return state;
}

function inferDepartment(parent, jobId) {
  if (['A01', 'A02', 'A11', 'A12', 'A13', 'A14', 'A23'].includes(parent)) return 'runtime-core';
  if (['A03', 'A04', 'A05', 'A06', 'A07', 'A08'].includes(parent)) return 'policy-security';
  if (['A15', 'A16', 'A17', 'A18', 'A25'].includes(parent)) return 'messaging';
  if (['A09', 'A10', 'A19'].includes(parent)) return 'endpoint-runner';
  if (['A20', 'A24'].includes(parent)) return 'ui-integration';
  if (['A21', 'A22'].includes(parent)) return 'packaging-distribution';
  if (['A26', 'A27', 'A28', 'A29'].includes(parent)) return 'memory-context';
  if (jobId === 'M36') return 'runtime-core';
  if (jobId === 'M37') return 'ui-integration';
  if (jobId === 'M38') return 'messaging';
  return 'unassigned';
}

function normalizeJobId(token) {
  const m = String(token || '').trim().match(/^M(\d+)$/i);
  if (!m) return null;
  return `M${m[1].padStart(2, '0')}`;
}

function expandMicroToken(token) {
  const t = String(token || '').trim();
  if (!t) return [];
  const range = t.match(/^M(\d+)\s*-\s*M(\d+)$/i);
  if (!range) {
    const single = normalizeJobId(t);
    return single ? [single] : [];
  }
  const from = Number(range[1]);
  const to = Number(range[2]);
  if (!Number.isFinite(from) || !Number.isFinite(to) || from > to) return [];
  const out = [];
  for (let n = from; n <= to; n += 1) {
    out.push(`M${String(n).padStart(2, '0')}`);
  }
  return out;
}

function parseBatches(planText) {
  const batches = {};
  const lines = planText.split('\n');
  for (const line of lines) {
    const m = line.match(/Batch\s+(\d+)\s*:\s*(.+)$/i);
    if (!m) continue;
    const batchNo = Number(m[1]);
    if (!Number.isFinite(batchNo)) continue;
    const rhs = m[2];
    const jobs = rhs
      .split(',')
      .flatMap((tok) => expandMicroToken(tok))
      .filter(Boolean);
    if (jobs.length > 0) {
      batches[batchNo] = jobs;
    }
  }
  return batches;
}

function parseMicroJobs(planPath) {
  const text = fs.readFileSync(planPath, 'utf8');
  const lines = text.split('\n');
  const jobs = [];
  for (const line of lines) {
    const m = line.match(/^\|\s*(M\d{2,})\s*\|\s*(EG-M\d{2,})\s*\|\s*(A\d{2})\s*\|\s*(.+?)\s*\|\s*$/);
    if (!m) continue;
    const [, rawId, endGate, parent, scope] = m;
    const id = normalizeJobId(rawId) || rawId;
    jobs.push({
      id,
      parent,
      endGate,
      scope,
      department: inferDepartment(parent, id),
      status: 'pending',
      attempts: 0,
      redoCount: 0,
      assignees: {},
      notes: [],
      events: [],
      createdAt: nowIso(),
      stage: {
        buildDone: false,
        reconcileDone: false,
        verifyResult: 'pending', // pending|pass|fail
        fixDone: false,
      },
      batch: null,
    });
  }
  return { jobs, planText: text };
}

function upsertEvent(state, type, payload) {
  state.events.push({ ts: nowIso(), type, ...payload });
}

function requireState(file) {
  const s = loadState(file);
  if (!s) {
    console.error(`State not found: ${file}`);
    console.error('Run: node scripts/agent-control.js init');
    process.exit(1);
  }
  return s;
}

function requireJob(state, jobId) {
  const j = state.jobs[jobId];
  if (!j) {
    console.error(`Unknown job: ${jobId}`);
    process.exit(1);
  }
  return j;
}

function closeAgentClock(job, agent, tsIso) {
  const slot = job.assignees[agent];
  if (!slot || !slot.activeSince) return;
  const delta = Math.max(0, asMs(tsIso) - asMs(slot.activeSince));
  slot.totalMs = (slot.totalMs || 0) + delta;
  slot.activeSince = null;
}

function totalAgentMs(job, tsIso) {
  let total = 0;
  for (const slot of Object.values(job.assignees)) {
    total += slot.totalMs || 0;
    if (slot.activeSince) {
      total += Math.max(0, asMs(tsIso) - asMs(slot.activeSince));
    }
  }
  return total;
}

function activeAgentCount(job) {
  return Object.values(job.assignees).filter((s) => Boolean(s.activeSince)).length;
}

function formatDuration(ms) {
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return `${h}h ${m}m ${sec}s`;
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

function listReadyJobs(state, phase = null) {
  const p = phase || state.pipeline?.phase || 'build';
  return Object.values(state.jobs).filter((job) => jobInCurrentBatch(state, job) && jobReadyForPhase(job, p));
}

function nextConcernId(state) {
  state.concernSeq = (state.concernSeq || 0) + 1;
  return `C${String(state.concernSeq).padStart(4, '0')}`;
}

function printConcerns(state, statusFilter = 'open', limit = 25) {
  const concerns = (state.concerns || [])
    .filter((c) => (statusFilter === 'all' ? true : c.status === statusFilter))
    .sort((a, b) => asMs(b.createdAt) - asMs(a.createdAt))
    .slice(0, limit);

  if (concerns.length === 0) {
    console.log('No concerns.');
    return;
  }
  for (const c of concerns) {
    console.log(`- ${c.id} [${c.status}] job=${c.jobId} agent=${c.agent} severity=${c.severity}`);
    console.log(`  q: ${c.text}`);
    if (c.response) console.log(`  guidance: ${c.response}`);
    if (c.action) console.log(`  action: ${c.action}`);
  }
}

function printRecentEvents(state, limit = 20) {
  const rows = (state.events || []).slice(-limit);
  if (rows.length === 0) {
    console.log('No events.');
    return;
  }
  for (const e of rows) {
    const payload = { ...e };
    delete payload.ts;
    const body = Object.entries(payload)
      .map(([k, v]) => `${k}=${typeof v === 'object' ? JSON.stringify(v) : String(v)}`)
      .join(' ');
    console.log(`- ${e.ts} ${body}`);
  }
}

function computeSummary(state, tsIso) {
  const jobs = Object.values(state.jobs);
  const total = jobs.length;
  const currentPhase = state.pipeline?.phase || 'build';
  const currentBatch = getCurrentBatch(state);
  const batchJobs = jobs.filter((j) => (currentBatch == null ? true : j.batch === currentBatch));
  const byStatus = {
    pending: 0,
    in_progress: 0,
    blocked: 0,
    failed: 0,
    done: 0,
    reconcile: 0,
  };
  for (const j of jobs) {
    byStatus[j.status] = (byStatus[j.status] || 0) + 1;
  }
  const byStage = {
    buildDone: 0,
    reconcileDone: 0,
    verifyPass: 0,
    verifyFail: 0,
    verifyPending: 0,
    fixDone: 0,
  };
  for (const j of jobs) {
    if (j.stage?.buildDone) byStage.buildDone += 1;
    if (j.stage?.reconcileDone) byStage.reconcileDone += 1;
    if (j.stage?.verifyResult === 'pass') byStage.verifyPass += 1;
    if (j.stage?.verifyResult === 'fail') byStage.verifyFail += 1;
    if (j.stage?.verifyResult === 'pending') byStage.verifyPending += 1;
    if (j.stage?.fixDone) byStage.fixDone += 1;
  }

  let phaseDone = 0;
  let phaseTotal = batchJobs.length;
  if (currentPhase === 'build') {
    phaseDone = batchJobs.filter((j) => j.stage?.buildDone).length;
  } else if (currentPhase === 'reconcile') {
    phaseDone = batchJobs.filter((j) => j.stage?.reconcileDone).length;
  } else if (currentPhase === 'verify') {
    phaseDone = batchJobs.filter((j) => j.stage?.verifyResult !== 'pending').length;
  } else if (currentPhase === 'fix') {
    const failed = batchJobs.filter((j) => j.stage?.verifyResult === 'fail');
    phaseTotal = failed.length;
    phaseDone = failed.filter((j) => j.stage?.fixDone).length;
  }
  const completed = byStatus.done;
  const elapsedMs = Math.max(1, asMs(tsIso) - asMs(state.createdAt));
  const completedPerHour = completed / (elapsedMs / 3600000);
  const remaining = total - completed;
  const etaHours = completedPerHour > 0 ? remaining / completedPerHour : null;

  const byDept = {};
  for (const j of jobs) {
    const d = j.department;
    if (!byDept[d]) {
      byDept[d] = { total: 0, done: 0, activeAgents: 0, blocked: 0, failed: 0, reconcile: 0 };
    }
    byDept[d].total += 1;
    if (j.status === 'done') byDept[d].done += 1;
    if (j.status === 'blocked') byDept[d].blocked += 1;
    if (j.status === 'failed') byDept[d].failed += 1;
    if (j.status === 'reconcile') byDept[d].reconcile += 1;
    byDept[d].activeAgents += activeAgentCount(j);
  }

  return {
    total,
    completed,
    remaining,
    elapsedMs,
    completedPerHour,
    etaHours,
    byStatus,
    byDept,
    byStage,
    phaseDone,
    phaseTotal,
  };
}

function printDashboard(state) {
  const tsIso = nowIso();
  const s = computeSummary(state, tsIso);
  const phase = state.pipeline?.phase || 'build';
  const batch = getCurrentBatch(state);
  const readyNow = listReadyJobs(state).length;
  const rc = state.pipeline?.runControl || { mode: 'running', pauseMode: null, reason: null };

  console.log('=== Agent Orchestration Dashboard ===');
  console.log(`State file: ${state.stateFile}`);
  console.log(`Started: ${state.createdAt}`);
  console.log(`Now:     ${tsIso}`);
  console.log(`Phase:   ${phase}`);
  console.log(`Batch:   ${batch == null ? 'n/a' : batch}`);
  console.log(`Run mode:${rc.mode}${rc.pauseMode ? ` (${rc.pauseMode})` : ''}${rc.reason ? ` reason=${rc.reason}` : ''}`);
  console.log(`Ready in phase: ${readyNow}`);
  console.log('');

  console.log(`Progress (final): ${s.completed}/${s.total} completed`);
  console.log(`Progress (phase): ${phase} ${s.phaseDone}/${s.phaseTotal} complete in batch ${batch == null ? 'n/a' : batch}`);
  console.log(`Status: pending=${s.byStatus.pending} in_progress=${s.byStatus.in_progress} blocked=${s.byStatus.blocked} failed=${s.byStatus.failed} reconcile=${s.byStatus.reconcile} done=${s.byStatus.done}`);
  console.log(`Stage totals: buildDone=${s.byStage.buildDone} reconcileDone=${s.byStage.reconcileDone} verifyPass=${s.byStage.verifyPass} verifyFail=${s.byStage.verifyFail} verifyPending=${s.byStage.verifyPending} fixDone=${s.byStage.fixDone}`);
  console.log(`Elapsed: ${formatDuration(s.elapsedMs)}`);
  console.log(`Throughput: ${s.completedPerHour.toFixed(2)} jobs/hour`);
  console.log(`ETA: ${s.etaHours == null ? 'n/a' : `${s.etaHours.toFixed(2)} hours`}`);
  console.log('');

  console.log('Departments:');
  for (const [dept, v] of Object.entries(s.byDept)) {
    console.log(`- ${dept}: done ${v.done}/${v.total}, activeAgents=${v.activeAgents}, blocked=${v.blocked}, failed=${v.failed}, reconcile=${v.reconcile}`);
  }
  console.log('');

  const problemJobs = Object.values(state.jobs).filter((j) => ['blocked', 'failed', 'reconcile'].includes(j.status));
  if (problemJobs.length > 0) {
    console.log('Problems/Reconciliation:');
    for (const j of problemJobs.slice(0, 20)) {
      const spent = formatDuration(totalAgentMs(j, tsIso));
      console.log(`- ${j.id} [${j.status}] dept=${j.department} batch=${j.batch ?? 'n/a'} activeAgents=${activeAgentCount(j)} spent=${spent} endGate=${j.endGate}`);
      if (j.lastReason) console.log(`  reason: ${j.lastReason}`);
    }
  } else {
    console.log('Problems/Reconciliation: none');
  }
  console.log('');

  const ready = listReadyJobs(state).slice(0, 20);
  console.log('Ready queue (current phase):');
  if (ready.length === 0) {
    console.log('- none');
  } else {
    for (const j of ready) {
      console.log(`- ${j.id} dept=${j.department} scope=${j.scope}`);
    }
  }
}

function usage() {
  console.log(`Usage:
  node scripts/agent-control.js init [--plan <path>] [--state-file <path>]
  node scripts/agent-control.js summary [--state-file <path>]
  node scripts/agent-control.js watch [--interval <sec>] [--state-file <path>]
  node scripts/agent-control.js monitor [--interval <sec>] [--state-file <path>]
  node scripts/agent-control.js run-state [--state-file <path>]
  node scripts/agent-control.js pause-run [--mode immediate|drain] [--reason <text>] [--state-file <path>]
  node scripts/agent-control.js resume-run [--state-file <path>]
  node scripts/agent-control.js pause-finalize [--state-file <path>]
  node scripts/agent-control.js phase [--state-file <path>]
  node scripts/agent-control.js set-phase <build|reconcile|verify|fix> [--state-file <path>]
  node scripts/agent-control.js advance [--state-file <path>]
  node scripts/agent-control.js ready [--limit <n>] [--state-file <path>]
  node scripts/agent-control.js start <jobId> --agent <agentName> [--state-file <path>]
  node scripts/agent-control.js pause <jobId> --agent <agentName> [--state-file <path>]
  node scripts/agent-control.js complete <jobId> [--note <text>] [--final] [--state-file <path>]
  node scripts/agent-control.js reconcile-done <jobId> [--note <text>] [--state-file <path>]
  node scripts/agent-control.js verify <jobId> --result pass|fail [--note <text>] [--state-file <path>]
  node scripts/agent-control.js fix-done <jobId> [--note <text>] [--state-file <path>]
  node scripts/agent-control.js fail <jobId> --reason <text> [--state-file <path>]
  node scripts/agent-control.js block <jobId> --reason <text> [--state-file <path>]
  node scripts/agent-control.js reconcile <jobId> --rstate start|done|redo [--note <text>] [--state-file <path>]
  node scripts/agent-control.js note <jobId> --text <text> [--state-file <path>]
  node scripts/agent-control.js concerns [--status open|closed|all] [--limit <n>] [--state-file <path>]
  node scripts/agent-control.js raise <jobId> --agent <name> --text <question> [--severity low|medium|high] [--state-file <path>]
  node scripts/agent-control.js guide <concernId> --response <text> [--action proceed|block|redo|unblock] [--state-file <path>]
  node scripts/agent-control.js events [--limit <n>] [--state-file <path>]
`);
}

function ensureJobReadyForCurrentPhase(state, jobId) {
  const job = requireJob(state, jobId);
  const phase = state.pipeline?.phase || 'build';
  if (!jobInCurrentBatch(state, job)) {
    console.error(`Job ${jobId} is not in current batch ${getCurrentBatch(state)} (job batch: ${job.batch ?? 'n/a'})`);
    process.exit(1);
  }
  if (!jobReadyForPhase(job, phase)) {
    console.error(`Job ${jobId} is not ready for current phase '${phase}'`);
    process.exit(1);
  }
  return job;
}

function tryAdvance(state) {
  const phase = state.pipeline.phase;
  const batch = getCurrentBatch(state);
  if (batch == null) {
    return { ok: false, message: 'No current batch' };
  }
  const batchJobs = Object.values(state.jobs).filter((j) => j.batch === batch);
  const all = (pred) => batchJobs.every(pred);

  if (phase === 'build') {
    if (!all((j) => j.stage.buildDone)) return { ok: false, message: 'Build phase not complete for current batch' };
    state.pipeline.phase = 'reconcile';
    return { ok: true, message: `Advanced to reconcile (batch ${batch})` };
  }
  if (phase === 'reconcile') {
    if (!all((j) => j.stage.reconcileDone)) return { ok: false, message: 'Reconcile phase not complete for current batch' };
    state.pipeline.phase = 'verify';
    return { ok: true, message: `Advanced to verify (batch ${batch})` };
  }
  if (phase === 'verify') {
    if (!all((j) => j.stage.verifyResult !== 'pending')) return { ok: false, message: 'Verify phase not complete for current batch' };
    const anyFailed = batchJobs.some((j) => j.stage.verifyResult === 'fail');
    if (anyFailed) {
      state.pipeline.phase = 'fix';
      return { ok: true, message: `Advanced to fix (batch ${batch})` };
    }
    const next = (state.pipeline.currentBatchIndex || 0) + 1;
    if (next >= state.pipeline.batchOrder.length) {
      state.pipeline.completed = true;
      return { ok: true, message: 'All batches completed (verify pass)' };
    }
    state.pipeline.currentBatchIndex = next;
    state.pipeline.phase = 'build';
    return { ok: true, message: `Moved to batch ${getCurrentBatch(state)} build phase` };
  }
  if (phase === 'fix') {
    const failedJobs = batchJobs.filter((j) => j.stage.verifyResult === 'fail');
    const fixedAll = failedJobs.every((j) => j.stage.fixDone);
    if (!fixedAll) return { ok: false, message: 'Fix phase not complete for failed jobs in current batch' };
    const next = (state.pipeline.currentBatchIndex || 0) + 1;
    if (next >= state.pipeline.batchOrder.length) {
      state.pipeline.completed = true;
      return { ok: true, message: 'All batches completed (after fixes)' };
    }
    state.pipeline.currentBatchIndex = next;
    state.pipeline.phase = 'build';
    return { ok: true, message: `Moved to batch ${getCurrentBatch(state)} build phase` };
  }
  return { ok: false, message: `Unknown phase ${phase}` };
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const cmd = args._[0];
  const stateFile = path.resolve(String(args['state-file'] || args.state || DEFAULT_STATE));

  if (!cmd || cmd === 'help' || cmd === '--help') {
    usage();
    process.exit(0);
  }

  if (cmd === 'init') {
    const plan = path.resolve(String(args.plan || DEFAULT_PLAN));
    if (!fs.existsSync(plan)) {
      console.error(`Plan file not found: ${plan}`);
      process.exit(1);
    }
    const { jobs, planText } = parseMicroJobs(plan);
    if (jobs.length === 0) {
      console.error(`No micro jobs parsed from ${plan}`);
      process.exit(1);
    }
    const parsedBatches = parseBatches(planText);
    const batchOrder = Object.keys(parsedBatches)
      .map((v) => Number(v))
      .filter((n) => Number.isFinite(n))
      .sort((a, b) => a - b);
    const batchMap = {};
    for (const b of batchOrder) {
      for (const jobId of parsedBatches[b]) {
        batchMap[jobId] = b;
      }
    }
    for (const j of jobs) {
      j.batch = batchMap[j.id] || null;
    }

    const state = {
      version: 2,
      createdAt: nowIso(),
      updatedAt: nowIso(),
      stateFile,
      sourcePlan: plan,
      events: [],
      pipeline: {
        phase: 'build',
        batchOrder,
        currentBatchIndex: 0,
        completed: false,
        runControl: {
          mode: 'running',
          pauseMode: null,
          reason: null,
          requestedAt: null,
          pausedAt: null,
        },
      },
      jobs: Object.fromEntries(jobs.map((j) => [j.id, j])),
    };
    upsertEvent(state, 'init', {
      totalJobs: jobs.length,
      sourcePlan: plan,
      batches: batchOrder.length,
    });
    saveState(stateFile, state);
    console.log(`Initialized state with ${jobs.length} jobs and ${batchOrder.length} batches -> ${stateFile}`);
    return;
  }

  const state = requireState(stateFile);
  migrateState(state);
  const ts = nowIso();

  if (cmd === 'summary') {
    printDashboard(state);
    return;
  }

  if (cmd === 'watch') {
    const intervalSec = Number(args.interval || 5);
    const intervalMs = Number.isFinite(intervalSec) && intervalSec > 0 ? intervalSec * 1000 : 5000;
    const loop = () => {
      const s = requireState(stateFile);
      process.stdout.write('\x1Bc');
      printDashboard(s);
    };
    loop();
    setInterval(loop, intervalMs);
    return;
  }

  if (cmd === 'monitor') {
    const intervalSec = Number(args.interval || 5);
    const intervalMs = Number.isFinite(intervalSec) && intervalSec > 0 ? intervalSec * 1000 : 5000;
    const loop = () => {
      const s = requireState(stateFile);
      migrateState(s);
      process.stdout.write('\x1Bc');
      printDashboard(s);
      console.log('');
      console.log('Open concerns:');
      printConcerns(s, 'open', 20);
      console.log('');
      console.log('Recent events:');
      printRecentEvents(s, 12);
      console.log('');
      console.log('Commands: raise | guide | concerns | ready | advance | pause-run | resume-run | run-state');
    };
    loop();
    setInterval(loop, intervalMs);
    return;
  }

  if (cmd === 'phase') {
    console.log(`Phase: ${state.pipeline?.phase || 'build'}`);
    console.log(`Batch: ${getCurrentBatch(state) ?? 'n/a'}`);
    console.log(`Completed: ${Boolean(state.pipeline?.completed)}`);
    return;
  }

  if (cmd === 'run-state') {
    const rc = state.pipeline?.runControl || {};
    console.log(`Run mode: ${rc.mode || 'running'}`);
    console.log(`Pause mode: ${rc.pauseMode || 'n/a'}`);
    console.log(`Requested at: ${rc.requestedAt || 'n/a'}`);
    console.log(`Paused at: ${rc.pausedAt || 'n/a'}`);
    console.log(`Reason: ${rc.reason || 'n/a'}`);
    return;
  }

  if (cmd === 'pause-run') {
    const mode = String(args.mode || 'drain').trim();
    const reason = String(args.reason || '').trim() || null;
    if (!['drain', 'immediate'].includes(mode)) {
      console.error('--mode must be drain|immediate');
      process.exit(1);
    }
    const rc = state.pipeline.runControl || {};
    if (mode === 'immediate') {
      state.pipeline.runControl = {
        mode: 'paused',
        pauseMode: 'immediate',
        reason,
        requestedAt: ts,
        pausedAt: ts,
      };
      upsertEvent(state, 'pipeline.pause', { mode: 'immediate', reason });
      saveState(stateFile, state);
      console.log('Run paused immediately.');
      return;
    }
    state.pipeline.runControl = {
      mode: 'pause_requested',
      pauseMode: 'drain',
      reason,
      requestedAt: ts,
      pausedAt: rc.pausedAt || null,
    };
    upsertEvent(state, 'pipeline.pause.requested', { mode: 'drain', reason });
    saveState(stateFile, state);
    console.log('Drain pause requested. Autopilot will stop dispatch and pause when active jobs reach zero.');
    return;
  }

  if (cmd === 'pause-finalize') {
    const rc = state.pipeline.runControl || {};
    if (rc.mode !== 'pause_requested') {
      console.log(`No-op: run mode is '${rc.mode || 'running'}' (expected pause_requested).`);
      return;
    }
    state.pipeline.runControl = {
      mode: 'paused',
      pauseMode: rc.pauseMode || 'drain',
      reason: rc.reason || null,
      requestedAt: rc.requestedAt || ts,
      pausedAt: ts,
    };
    upsertEvent(state, 'pipeline.pause.finalized', {
      mode: state.pipeline.runControl.pauseMode,
      reason: state.pipeline.runControl.reason,
    });
    saveState(stateFile, state);
    console.log('Run paused (drain complete).');
    return;
  }

  if (cmd === 'resume-run') {
    const prev = state.pipeline.runControl || {};
    state.pipeline.runControl = {
      mode: 'running',
      pauseMode: null,
      reason: null,
      requestedAt: null,
      pausedAt: null,
    };
    upsertEvent(state, 'pipeline.resumed', { from: prev.mode || 'unknown' });
    saveState(stateFile, state);
    console.log('Run resumed.');
    return;
  }

  if (cmd === 'set-phase') {
    const p = String(args._[1] || '').trim();
    if (!PHASES.includes(p)) {
      console.error('Phase must be one of: build|reconcile|verify|fix');
      process.exit(1);
    }
    state.pipeline.phase = p;
    upsertEvent(state, 'pipeline.phase.set', { phase: p });
    saveState(stateFile, state);
    console.log(`Set phase to ${p}`);
    return;
  }

  if (cmd === 'advance') {
    const r = tryAdvance(state);
    if (!r.ok) {
      console.error(r.message);
      process.exit(1);
    }
    upsertEvent(state, 'pipeline.advance', { phase: state.pipeline.phase, batch: getCurrentBatch(state) });
    saveState(stateFile, state);
    console.log(r.message);
    return;
  }

  if (cmd === 'ready') {
    const limit = Number(args.limit || 20);
    const rows = listReadyJobs(state).slice(0, Number.isFinite(limit) ? limit : 20);
    if (rows.length === 0) {
      console.log('No ready jobs in current phase.');
      return;
    }
    for (const j of rows) {
      console.log(`${j.id}\tbatch=${j.batch ?? 'n/a'}\tdept=${j.department}\t${j.scope}`);
    }
    return;
  }

  if (cmd === 'concerns') {
    const status = String(args.status || 'open').trim();
    const allowed = ['open', 'closed', 'all'];
    if (!allowed.includes(status)) {
      console.error('--status must be open|closed|all');
      process.exit(1);
    }
    const limit = Number(args.limit || 25);
    printConcerns(state, status, Number.isFinite(limit) ? limit : 25);
    return;
  }

  if (cmd === 'events') {
    const limit = Number(args.limit || 30);
    printRecentEvents(state, Number.isFinite(limit) ? limit : 30);
    return;
  }

  if (cmd === 'guide') {
    const concernId = String(args._[1] || '').trim();
    if (!concernId) {
      console.error('Missing <concernId>');
      process.exit(1);
    }
    const concern = (state.concerns || []).find((c) => c.id === concernId);
    if (!concern) {
      console.error(`Unknown concern: ${concernId}`);
      process.exit(1);
    }
    const response = String(args.response || '').trim();
    if (!response) {
      console.error('--response is required');
      process.exit(1);
    }
    const action = args.action ? String(args.action).trim() : null;
    if (action && !['proceed', 'block', 'redo', 'unblock'].includes(action)) {
      console.error('--action must be proceed|block|redo|unblock');
      process.exit(1);
    }
    concern.status = 'closed';
    concern.closedAt = ts;
    concern.response = response;
    concern.action = action;

    const targetJob = requireJob(state, concern.jobId);
    if (action === 'block') targetJob.status = 'blocked';
    if (action === 'redo') {
      targetJob.status = 'pending';
      targetJob.redoCount = (targetJob.redoCount || 0) + 1;
    }
    if (action === 'unblock' && targetJob.status === 'blocked') targetJob.status = 'pending';

    upsertEvent(state, 'concern.guided', { concernId, jobId: concern.jobId, action });
    saveState(stateFile, state);
    console.log(`Guidance applied to ${concernId}`);
    return;
  }

  const jobId = args._[1];
  if (!jobId && cmd !== 'guide') {
    console.error('Missing <jobId>');
    usage();
    process.exit(1);
  }
  const normalizedJobId = jobId ? (normalizeJobId(jobId) || jobId) : null;
  const job = normalizedJobId ? requireJob(state, normalizedJobId) : null;

  if (cmd === 'raise') {
    const agent = String(args.agent || '').trim();
    const text = String(args.text || '').trim();
    const severity = String(args.severity || 'medium').trim();
    if (!agent) {
      console.error('--agent is required');
      process.exit(1);
    }
    if (!text) {
      console.error('--text is required');
      process.exit(1);
    }
    if (!['low', 'medium', 'high'].includes(severity)) {
      console.error('--severity must be low|medium|high');
      process.exit(1);
    }
    const id = nextConcernId(state);
    const concern = {
      id,
      jobId: normalizedJobId,
      agent,
      text,
      severity,
      status: 'open',
      createdAt: ts,
      closedAt: null,
      response: null,
      action: null,
    };
    state.concerns.push(concern);
    upsertEvent(state, 'concern.raised', { concernId: id, jobId: normalizedJobId, agent, severity });
    saveState(stateFile, state);
    console.log(`Concern raised: ${id}`);
    return;
  }

  if (cmd === 'start') {
    const readyJob = ensureJobReadyForCurrentPhase(state, normalizedJobId);
    const agent = String(args.agent || '').trim();
    if (!agent) {
      console.error('--agent is required');
      process.exit(1);
    }
    if (!readyJob.assignees[agent]) {
      readyJob.assignees[agent] = { totalMs: 0, activeSince: null };
    }
    if (!readyJob.assignees[agent].activeSince) {
      readyJob.assignees[agent].activeSince = ts;
    }
    if (readyJob.status !== 'in_progress') {
      readyJob.status = 'in_progress';
      readyJob.attempts += 1;
    }
    upsertEvent(state, 'job.start', { jobId: normalizedJobId, agent, phase: state.pipeline.phase });
    saveState(stateFile, state);
    console.log(`Started ${normalizedJobId} with agent ${agent} in phase ${state.pipeline.phase}`);
    return;
  }

  if (cmd === 'pause') {
    const agent = String(args.agent || '').trim();
    if (!agent) {
      console.error('--agent is required');
      process.exit(1);
    }
    closeAgentClock(job, agent, ts);
    if (activeAgentCount(job) === 0 && job.status === 'in_progress') {
      job.status = 'pending';
    }
    upsertEvent(state, 'job.pause', { jobId: normalizedJobId, agent });
    saveState(stateFile, state);
    console.log(`Paused ${normalizedJobId} for agent ${agent}`);
    return;
  }

  if (cmd === 'complete') {
    for (const agent of Object.keys(job.assignees)) {
      closeAgentClock(job, agent, ts);
    }
    const note = args.note ? String(args.note) : null;
    const forceFinal = Boolean(args.final);
    const phase = state.pipeline?.phase || 'build';

    if (forceFinal) {
      job.status = 'done';
    } else if (phase === 'build') {
      job.stage.buildDone = true;
      job.status = 'pending';
    } else if (phase === 'fix') {
      job.stage.fixDone = true;
      job.status = 'done';
    } else {
      job.status = 'pending';
    }
    if (note) {
      job.notes.push({ ts, text: note });
    }
    upsertEvent(state, 'job.complete', { jobId: normalizedJobId, phase, forceFinal });
    saveState(stateFile, state);
    console.log(`Completed ${normalizedJobId} (${forceFinal ? 'final' : `phase=${phase}`})`);
    return;
  }

  if (cmd === 'reconcile-done') {
    if (state.pipeline.phase !== 'reconcile') {
      console.error(`Current phase is '${state.pipeline.phase}', expected 'reconcile'`);
      process.exit(1);
    }
    ensureJobReadyForCurrentPhase(state, normalizedJobId);
    for (const agent of Object.keys(job.assignees)) {
      closeAgentClock(job, agent, ts);
    }
    job.stage.reconcileDone = true;
    job.status = 'pending';
    if (args.note) job.notes.push({ ts, text: `[reconcile] ${String(args.note)}` });
    upsertEvent(state, 'job.reconcile.done', { jobId: normalizedJobId });
    saveState(stateFile, state);
    console.log(`Reconcile done: ${normalizedJobId}`);
    return;
  }

  if (cmd === 'verify') {
    if (state.pipeline.phase !== 'verify') {
      console.error(`Current phase is '${state.pipeline.phase}', expected 'verify'`);
      process.exit(1);
    }
    ensureJobReadyForCurrentPhase(state, normalizedJobId);
    const result = String(args.result || '').trim();
    if (!['pass', 'fail'].includes(result)) {
      console.error('--result must be pass|fail');
      process.exit(1);
    }
    for (const agent of Object.keys(job.assignees)) {
      closeAgentClock(job, agent, ts);
    }
    job.stage.verifyResult = result;
    job.status = result === 'pass' ? 'done' : 'failed';
    if (args.note) job.notes.push({ ts, text: `[verify:${result}] ${String(args.note)}` });
    upsertEvent(state, 'job.verify', { jobId: normalizedJobId, result });
    saveState(stateFile, state);
    console.log(`Verify ${normalizedJobId}: ${result}`);
    return;
  }

  if (cmd === 'fix-done') {
    if (state.pipeline.phase !== 'fix') {
      console.error(`Current phase is '${state.pipeline.phase}', expected 'fix'`);
      process.exit(1);
    }
    ensureJobReadyForCurrentPhase(state, normalizedJobId);
    for (const agent of Object.keys(job.assignees)) {
      closeAgentClock(job, agent, ts);
    }
    job.stage.fixDone = true;
    job.status = 'done';
    if (args.note) job.notes.push({ ts, text: `[fix] ${String(args.note)}` });
    upsertEvent(state, 'job.fix.done', { jobId: normalizedJobId });
    saveState(stateFile, state);
    console.log(`Fix done: ${normalizedJobId}`);
    return;
  }

  if (cmd === 'fail') {
    const reason = String(args.reason || '').trim();
    if (!reason) {
      console.error('--reason is required');
      process.exit(1);
    }
    for (const agent of Object.keys(job.assignees)) {
      closeAgentClock(job, agent, ts);
    }
    job.status = 'failed';
    job.lastReason = reason;
    upsertEvent(state, 'job.fail', { jobId: normalizedJobId, reason });
    saveState(stateFile, state);
    console.log(`Failed ${normalizedJobId}: ${reason}`);
    return;
  }

  if (cmd === 'block') {
    const reason = String(args.reason || '').trim();
    if (!reason) {
      console.error('--reason is required');
      process.exit(1);
    }
    for (const agent of Object.keys(job.assignees)) {
      closeAgentClock(job, agent, ts);
    }
    job.status = 'blocked';
    job.lastReason = reason;
    upsertEvent(state, 'job.block', { jobId: normalizedJobId, reason });
    saveState(stateFile, state);
    console.log(`Blocked ${normalizedJobId}: ${reason}`);
    return;
  }

  if (cmd === 'reconcile') {
    const rstate = String(args.rstate || args['reconcile-state'] || '').trim();
    if (!['start', 'done', 'redo'].includes(rstate)) {
      console.error('--rstate must be start|done|redo');
      process.exit(1);
    }
    const note = args.note ? String(args.note) : null;
    if (rstate === 'start') {
      job.status = 'reconcile';
    } else if (rstate === 'done') {
      if (job.status !== 'done') {
        job.status = 'pending';
      }
    } else if (rstate === 'redo') {
      for (const agent of Object.keys(job.assignees)) {
        closeAgentClock(job, agent, ts);
      }
      job.status = 'pending';
      job.redoCount += 1;
    }
    if (note) {
      job.notes.push({ ts, text: `[reconcile:${rstate}] ${note}` });
    }
    upsertEvent(state, 'job.reconcile', { jobId: normalizedJobId, state: rstate, note });
    saveState(stateFile, state);
    console.log(`Reconcile ${normalizedJobId}: ${rstate}`);
    return;
  }

  if (cmd === 'note') {
    const text = String(args.text || '').trim();
    if (!text) {
      console.error('--text is required');
      process.exit(1);
    }
    job.notes.push({ ts, text });
    upsertEvent(state, 'job.note', { jobId: normalizedJobId, text });
    saveState(stateFile, state);
    console.log(`Noted ${normalizedJobId}`);
    return;
  }

  console.error(`Unknown command: ${cmd}`);
  usage();
  process.exit(1);
}

main();
