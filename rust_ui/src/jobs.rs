//! Port of server-jobs.js — agentic job queue.
//!
//! One FIFO worker (agent turns are heavyweight and share one CLI), a job
//! record with live progress, history, and cancellation. Every long-running
//! invocation — chat turns, skill generation — goes through here.

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::{base36, now_ms};

const HISTORY: usize = 50;
const PROGRESS_CAP: usize = 20;

pub type CancelFn = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Queued,
    Running,
    Done,
    Error,
    Cancelled,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressEntry {
    pub t: u64,
    pub detail: String,
}

/// Events streamed to subscribers (the NDJSON /api/agent?stream=1 bridge).
#[derive(Clone, Debug)]
pub enum JobEvent {
    Progress(String),
    Done,
}

/// Structured handler failure — mirrors the JS Error object with ad-hoc
/// `status` / `draft` / `raw` attachments (used by /api/skills/generate).
#[derive(Clone, Debug)]
pub struct JobError {
    pub message: String,
    pub status: Option<u16>,
    pub draft: Option<Value>,
    pub raw: Option<String>,
}

impl JobError {
    pub fn msg(s: impl Into<String>) -> JobError {
        JobError { message: s.into(), status: None, draft: None, raw: None }
    }
}

impl From<String> for JobError {
    fn from(s: String) -> Self {
        JobError::msg(s)
    }
}
impl From<&str> for JobError {
    fn from(s: &str) -> Self {
        JobError::msg(s)
    }
}

pub struct Job {
    pub id: String,
    pub kind: String,
    pub payload: Value,
    pub status: Status,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub ended_at: Option<u64>,
    pub progress: Vec<ProgressEntry>,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub error_status: Option<u16>,
    pub error_draft: Option<Value>,
    pub error_raw: Option<String>,
    subscribers: Vec<mpsc::UnboundedSender<JobEvent>>,
    cancel: Option<CancelFn>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PublicJob {
    pub id: String,
    pub kind: String,
    pub status: Status,
    #[serde(rename = "createdAt")]
    pub created_at: u64,
    #[serde(rename = "startedAt")]
    pub started_at: Option<u64>,
    #[serde(rename = "endedAt")]
    pub ended_at: Option<u64>,
    pub progress: Vec<ProgressEntry>,
    pub error: Option<String>,
    pub summary: String,
}

fn summary_of(j: &Job) -> String {
    if j.status == Status::Done {
        if let Some(r) = &j.result {
            if j.kind == "agent" {
                if let Some(reply) = r.get("reply").and_then(Value::as_str) {
                    return reply.chars().take(200).collect();
                }
            }
            if j.kind == "generate" {
                if let Some(name) = r.get("draft").and_then(|d| d.get("name")).and_then(Value::as_str) {
                    return format!("draft: {}", name);
                }
            }
        }
    }
    let payload_field = if j.kind == "agent" { "message" } else { "hint" };
    j.payload
        .get(payload_field)
        .and_then(Value::as_str)
        .map(|s| s.chars().take(120).collect())
        .unwrap_or_default()
}

impl Job {
    fn public(&self) -> PublicJob {
        PublicJob {
            id: self.id.clone(),
            kind: self.kind.clone(),
            status: self.status.clone(),
            created_at: self.created_at,
            started_at: self.started_at,
            ended_at: self.ended_at,
            progress: self.progress.iter().rev().take(5).rev().cloned().collect(),
            error: self.error.clone(),
            summary: summary_of(self),
        }
    }

    fn broadcast(&mut self, ev: JobEvent) {
        self.subscribers.retain(|tx| tx.send(ev.clone()).is_ok());
    }
}

type Handler = Arc<dyn Fn(JobCtx) -> HandlerFuture + Send + Sync>;
type HandlerFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, JobError>> + Send>>;

struct Inner {
    jobs: Mutex<HashMap<String, Job>>,
    queue: Mutex<VecDeque<String>>,
    running: Mutex<bool>,
    handlers: Mutex<HashMap<String, Handler>>,
    seq: AtomicU64,
}

#[derive(Clone)]
pub struct JobQueue {
    inner: Arc<Inner>,
}

/// The handle a handler receives: progress notes, cancellation hook, status.
#[derive(Clone)]
pub struct JobCtx {
    queue: JobQueue,
    pub id: String,
}

impl JobCtx {
    pub fn note(&self, detail: &str) {
        self.queue.note(&self.id, detail);
    }
    pub fn set_cancel(&self, f: CancelFn) {
        if let Ok(mut jobs) = self.queue.inner.jobs.lock() {
            if let Some(j) = jobs.get_mut(&self.id) {
                j.cancel = Some(f);
            }
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.queue
            .inner
            .jobs
            .lock()
            .ok()
            .and_then(|jobs| jobs.get(&self.id).map(|j| j.status == Status::Cancelled))
            .unwrap_or(false)
    }
    pub fn payload(&self) -> Value {
        self.queue
            .inner
            .jobs
            .lock()
            .ok()
            .and_then(|jobs| jobs.get(&self.id).map(|j| j.payload.clone()))
            .unwrap_or(Value::Null)
    }
}

impl Default for JobQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl JobQueue {
    pub fn new() -> JobQueue {
        JobQueue {
            inner: Arc::new(Inner {
                jobs: Mutex::new(HashMap::new()),
                queue: Mutex::new(VecDeque::new()),
                running: Mutex::new(false),
                handlers: Mutex::new(HashMap::new()),
                seq: AtomicU64::new(0),
            }),
        }
    }

    pub fn register_handler<F, Fut>(&self, kind: &str, f: F)
    where
        F: Fn(JobCtx) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Value, JobError>> + Send + 'static,
    {
        let handler: Handler = Arc::new(move |ctx| Box::pin(f(ctx)));
        self.inner.handlers.lock().unwrap().insert(kind.to_string(), handler);
    }

    fn trim_history(&self) {
        let mut jobs = self.inner.jobs.lock().unwrap();
        let mut all: Vec<(String, u64, Status)> = jobs
            .values()
            .map(|j| (j.id.clone(), j.created_at, j.status.clone()))
            .collect();
        all.sort_by(|a, b| b.1.cmp(&a.1));
        for (id, _, status) in all.into_iter().skip(HISTORY) {
            if status != Status::Queued && status != Status::Running {
                jobs.remove(&id);
            }
        }
    }

    pub fn submit(&self, kind: &str, payload: Value) -> Result<String, String> {
        if !self.inner.handlers.lock().unwrap().contains_key(kind) {
            return Err(format!("no handler for job kind {}", kind));
        }
        let seq = self.inner.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let id = format!("job-{}-{}", seq, base36(now_ms()));
        let job = Job {
            id: id.clone(),
            kind: kind.to_string(),
            payload: if payload.is_null() { json!({}) } else { payload },
            status: Status::Queued,
            created_at: now_ms(),
            started_at: None,
            ended_at: None,
            progress: Vec::new(),
            result: None,
            error: None,
            error_status: None,
            error_draft: None,
            error_raw: None,
            subscribers: Vec::new(),
            cancel: None,
        };
        self.inner.jobs.lock().unwrap().insert(id.clone(), job);
        self.inner.queue.lock().unwrap().push_back(id.clone());
        self.trim_history();
        crate::log::info("job.submit", &[("id", json!(id)), ("kind", json!(kind))]);
        self.pump();
        Ok(id)
    }

    /// JS pump(): synchronously flips the next queued job to `running`
    /// (startedAt set) before the handler's first await — callers of submit
    /// therefore observe "running" when the worker was idle.
    fn pump(&self) {
        {
            let mut running = self.inner.running.lock().unwrap();
            if *running {
                return;
            }
            let next = self.inner.queue.lock().unwrap().pop_front();
            let Some(id) = next else { return };
            {
                let mut jobs = self.inner.jobs.lock().unwrap();
                let Some(job) = jobs.get_mut(&id) else {
                    drop(jobs);
                    drop(running);
                    return self.pump();
                };
                if job.status == Status::Cancelled {
                    job.ended_at = Some(now_ms());
                    job.broadcast(JobEvent::Done);
                    drop(jobs);
                    drop(running);
                    return self.pump();
                }
                *running = true;
                job.status = Status::Running;
                job.started_at = Some(now_ms());
                crate::log::info(
                    "job.start",
                    &[
                        ("id", json!(id)),
                        ("kind", json!(job.kind)),
                        ("queuedMs", json!(job.started_at.unwrap() - job.created_at)),
                    ],
                );
            }
            let kind = self.inner.jobs.lock().unwrap().get(&id).map(|j| j.kind.clone());
            let Some(kind) = kind else { return };
            let handler = self.inner.handlers.lock().unwrap().get(&kind).cloned();
            let Some(handler) = handler else {
                *running = false;
                return;
            };
            let me = self.clone();
            tokio::spawn(async move {
                me.finish_job(&id, kind, handler).await;
                *me.inner.running.lock().unwrap() = false;
                me.pump();
            });
        }
    }

    async fn finish_job(&self, id: &str, kind: String, handler: Handler) {
        let ctx = JobCtx { queue: self.clone(), id: id.to_string() };
        let outcome = handler(ctx).await;
        let mut jobs = self.inner.jobs.lock().unwrap();
        if let Some(job) = jobs.get_mut(id) {
            match outcome {
                Ok(result) => {
                    job.result = Some(result);
                    if job.status != Status::Cancelled {
                        job.status = Status::Done;
                    }
                }
                Err(e) => {
                    if job.status != Status::Cancelled {
                        crate::log::warn(
                            "job.error",
                            &[("id", json!(id)), ("kind", json!(kind)), ("error", json!(e.message.chars().take(200).collect::<String>()))],
                        );
                        job.error = Some(e.message);
                        job.error_status = e.status;
                        job.error_draft = e.draft;
                        job.error_raw = e.raw;
                        job.status = Status::Error;
                    }
                }
            }
            job.ended_at = Some(now_ms());
            let ms = job.ended_at.unwrap() - job.started_at.unwrap_or(job.ended_at.unwrap());
            crate::log::info(
                "job.end",
                &[
                    ("id", json!(id)),
                    ("kind", json!(kind)),
                    ("status", json!(serde_json::to_string(&job.status).unwrap_or_default())),
                    ("ms", json!(ms)),
                ],
            );
            job.broadcast(JobEvent::Done);
            job.subscribers.clear();
        }
    }

    pub fn note(&self, id: &str, detail: &str) {
        if let Ok(mut jobs) = self.inner.jobs.lock() {
            if let Some(j) = jobs.get_mut(id) {
                j.progress.push(ProgressEntry { t: now_ms(), detail: detail.to_string() });
                if j.progress.len() > PROGRESS_CAP {
                    let start = j.progress.len() - PROGRESS_CAP;
                    j.progress.drain(0..start);
                }
                j.broadcast(JobEvent::Progress(detail.to_string()));
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<PublicJob> {
        self.inner.jobs.lock().ok()?.get(id).map(|j| j.public())
    }

    /// Full detail for GET /api/jobs/<id>: complete progress; `result` only
    /// present when done (the JS omits the key otherwise).
    pub fn detail(&self, id: &str) -> Option<Value> {
        let jobs = self.inner.jobs.lock().ok()?;
        let j = jobs.get(id)?;
        let mut out = serde_json::Map::new();
        out.insert("job".into(), json!(j.public()));
        out.insert("progress".into(), json!(j.progress));
        if j.status == Status::Done {
            out.insert("result".into(), j.result.clone().unwrap_or(Value::Null));
        }
        out.insert("error".into(), json!(j.error));
        Some(Value::Object(out))
    }

    pub fn list(&self) -> Vec<PublicJob> {
        let Ok(jobs) = self.inner.jobs.lock() else { return Vec::new() };
        let mut all: Vec<&Job> = jobs.values().collect();
        all.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        all.into_iter().map(|j| j.public()).collect()
    }

    pub fn cancel(&self, id: &str) -> Option<PublicJob> {
        let mut jobs = self.inner.jobs.lock().ok()?;
        let job = jobs.get_mut(id)?;
        match job.status {
            Status::Queued => {
                job.status = Status::Cancelled;
                job.ended_at = Some(now_ms());
                self.inner.queue.lock().unwrap().retain(|x| x != id);
                job.broadcast(JobEvent::Done);
            }
            Status::Running => {
                job.status = Status::Cancelled; // handler checks / process gets killed
                if let Some(cancel) = job.cancel.clone() {
                    cancel();
                }
                job.error = Some("cancelled by user".into());
                job.ended_at = Some(now_ms());
                job.broadcast(JobEvent::Done);
            }
            _ => {}
        }
        crate::log::info(
            "job.cancel",
            &[("id", json!(id)), ("status", json!(serde_json::to_string(&job.status).unwrap_or_default()))],
        );
        Some(job.public())
    }

    /// Subscribe to a job's progress/done events (for NDJSON streaming).
    /// Returns the receiver plus whether the job was already terminal at
    /// subscription time (atomic vs broadcasts — no missed Done).
    pub fn subscribe(&self, id: &str) -> Option<(mpsc::UnboundedReceiver<JobEvent>, bool)> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut jobs = self.inner.jobs.lock().ok()?;
        let job = jobs.get_mut(id)?;
        let terminal = matches!(job.status, Status::Done | Status::Error | Status::Cancelled);
        if !terminal {
            job.subscribers.push(tx);
        }
        Some((rx, terminal))
    }

    /// Current outcome snapshot (non-blocking).
    pub fn outcome(&self, id: &str) -> Option<JobOutcome> {
        let jobs = self.inner.jobs.lock().ok()?;
        let j = jobs.get(id)?;
        Some(JobOutcome {
            status: j.status.clone(),
            result: j.result.clone(),
            error: j.error.clone(),
            error_status: j.error_status,
            error_draft: j.error_draft.clone(),
            error_raw: j.error_raw.clone(),
        })
    }

    /// Wait for a job to reach a terminal status; returns its outcome.
    /// Err("job timed out") when the timeout elapses first.
    pub async fn wait_job(&self, id: &str, timeout: std::time::Duration) -> Result<JobOutcome, String> {
        let Some((mut rx, terminal)) = self.subscribe(id) else {
            return Err(format!("no job {}", id));
        };
        if !terminal {
            let hit = tokio::time::timeout(timeout, async {
                while let Some(ev) = rx.recv().await {
                    if matches!(ev, JobEvent::Done) {
                        break;
                    }
                }
            })
            .await
            .is_err();
            if hit {
                return Err("job timed out".into());
            }
        }
        self.outcome(id).ok_or_else(|| format!("no job {}", id))
    }
}

/// Terminal state of a job, as returned by `wait_job`.
#[derive(Clone, Debug)]
pub struct JobOutcome {
    pub status: Status,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub error_status: Option<u16>,
    pub error_draft: Option<Value>,
    pub error_raw: Option<String>,
}
