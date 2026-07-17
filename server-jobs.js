/* QPT Workbench — agentic job queue.
 *
 * One FIFO worker (agent turns are heavyweight and share one CLI), a job record
 * with live progress, history, and cancellation. Every long-running invocation —
 * chat turns, skill generation, anything future — goes through here.
 *
 * Job: { id, kind, status: queued|running|done|error|cancelled,
 *        createdAt, startedAt, endedAt, progress[], result, error, emitter }
 */
import { EventEmitter } from "node:events";
import { log } from "./server-log.js";

const HANDLERS = {};
const jobs = new Map();
const queue = [];
let running = false;
let seq = 0;
const HISTORY = 50;
const listeners = new Set(); // for a global "jobs changed" feed (badge etc.)

export function registerHandler(kind, fn) { HANDLERS[kind] = fn; }

function broadcast() { listeners.forEach((fn) => { try { fn(); } catch {} }); }
export function onJobsChanged(fn) { listeners.add(fn); return () => listeners.delete(fn); }

function trimHistory() {
  const all = [...jobs.values()].sort((a, b) => b.createdAt - a.createdAt);
  all.slice(HISTORY).forEach((j) => { if (j.status !== "queued" && j.status !== "running") jobs.delete(j.id); });
}

export function submitJob(kind, payload) {
  if (!HANDLERS[kind]) throw new Error("no handler for job kind " + kind);
  const job = {
    id: "job-" + ++seq + "-" + Date.now().toString(36),
    kind,
    payload: payload || {},
    status: "queued",
    createdAt: Date.now(),
    startedAt: null,
    endedAt: null,
    progress: [],
    result: null,
    error: null,
    emitter: new EventEmitter(),
    _cancel: null,
  };
  job.emitter.setMaxListeners(50);
  jobs.set(job.id, job);
  queue.push(job.id);
  trimHistory();
  log.info("job.submit", { id: job.id, kind });
  broadcast();
  pump();
  return job;
}

export function note(job, detail) {
  job.progress.push({ t: Date.now(), detail });
  if (job.progress.length > 20) job.progress = job.progress.slice(-20);
  job.emitter.emit("progress", detail);
  broadcast();
}

async function pump() {
  if (running) return;
  const id = queue.shift();
  if (!id) return;
  const job = jobs.get(id);
  if (!job) return pump();
  if (job.status === "cancelled") { job.endedAt = Date.now(); job.emitter.emit("done", job); return pump(); }
  running = true;
  job.status = "running";
  job.startedAt = Date.now();
  log.info("job.start", { id: job.id, kind: job.kind, queuedMs: job.startedAt - job.createdAt });
  broadcast();
  try {
    job.result = await HANDLERS[job.kind](job);
    if (job.status !== "cancelled") job.status = "done";
  } catch (e) {
    if (job.status !== "cancelled") {
      job.error = String((e && e.message) || e);
      job.status = "error";
      log.warn("job.error", { id: job.id, kind: job.kind, error: job.error.slice(0, 200) });
    }
  }
  job.endedAt = Date.now();
  log.info("job.end", { id: job.id, kind: job.kind, status: job.status, ms: job.endedAt - job.startedAt });
  job.emitter.emit("done", job);
  broadcast();
  running = false;
  pump();
}

export function getJob(id) { return jobs.get(id); }

export function cancelJob(id) {
  const job = jobs.get(id);
  if (!job) return null;
  if (job.status === "queued") {
    job.status = "cancelled";
    job.endedAt = Date.now();
    const i = queue.indexOf(id);
    if (i >= 0) queue.splice(i, 1);
    job.emitter.emit("done", job);
  } else if (job.status === "running") {
    job.status = "cancelled"; // handler checks / process gets killed
    if (job._cancel) {
      try { job._cancel(); } catch {}
    }
    job.error = "cancelled by user";
    job.endedAt = Date.now();
    job.emitter.emit("done", job);
  }
  log.info("job.cancel", { id, status: job.status });
  broadcast();
  return job;
}

function publicJob(j) {
  return {
    id: j.id,
    kind: j.kind,
    status: j.status,
    createdAt: j.createdAt,
    startedAt: j.startedAt,
    endedAt: j.endedAt,
    progress: j.progress.slice(-5),
    error: j.error,
    summary: summaryOf(j),
  };
}

function summaryOf(j) {
  if (j.status === "done" && j.result) {
    if (j.kind === "agent" && j.result.reply) return j.result.reply.slice(0, 200);
    if (j.kind === "generate" && j.result.draft) return "draft: " + j.result.draft.name;
  }
  if (j.kind === "agent" && j.payload && j.payload.message) return j.payload.message.slice(0, 120);
  if (j.kind === "generate" && j.payload && j.payload.hint) return j.payload.hint.slice(0, 120);
  return "";
}

export function listJobs() {
  return [...jobs.values()]
    .sort((a, b) => b.createdAt - a.createdAt)
    .map(publicJob);
}
