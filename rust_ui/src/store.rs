//! Port of server-store.js — canonical card store persisted to
//! data/store.json (atomic writes, version counter for sync). All mutations
//! go through `core`, so the store enforces exactly the semantics the
//! browser does.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::PathBuf;

use crate::{core, skills, Paths};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoreData {
    #[serde(default)]
    pub version: i64,
    #[serde(rename = "boardId", default = "default_board")]
    pub board_id: String,
    #[serde(rename = "customSeq", default)]
    pub custom_seq: i64,
    #[serde(default)]
    pub cards: Map<String, Value>,
}

fn default_board() -> String {
    "protocol".into()
}

pub struct Store {
    pub data: StoreData,
    pub file: PathBuf,
    paths: Paths,
    flush_tx: std::sync::mpsc::Sender<String>,
    flush_thread: Option<std::thread::JoinHandle<()>>,
}

/// Serialize the way the Node store does: JSON.stringify(data, null, 1).
fn to_json_indent1<T: Serialize>(value: &T) -> String {
    crate::json_indent1(value)
}

/// Atomic write: tmp file + rename. Write errors are dropped, as before.
fn write_store_file(file: &std::path::Path, contents: &str) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = file.with_extension("json.tmp");
    if std::fs::write(&tmp, contents).is_ok() {
        let _ = std::fs::rename(&tmp, file);
    }
}

/// One background flusher owns the store file: `persist` serializes in
/// memory and hands the String over in order; the flusher performs the
/// actual writes sequentially — write-through, last write wins, and no sync
/// fs calls in async contexts. A std thread (not a tokio task) so the store
/// also works without a runtime (qpt-cli). If the thread cannot start, the
/// receiver is dropped and sends become no-ops.
fn spawn_flusher(file: PathBuf) -> (std::sync::mpsc::Sender<String>, Option<std::thread::JoinHandle<()>>) {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let handle = std::thread::Builder::new()
        .name("store-flush".into())
        .spawn(move || {
            while let Ok(contents) = rx.recv() {
                write_store_file(&file, &contents);
            }
        })
        .ok();
    (tx, handle)
}

impl Drop for Store {
    fn drop(&mut self) {
        // Close the channel so the flusher drains queued writes and exits,
        // then wait for it — no lost writes on shutdown.
        let (idle, _rx) = std::sync::mpsc::channel::<String>();
        drop(std::mem::replace(&mut self.flush_tx, idle));
        if let Some(h) = self.flush_thread.take() {
            let _ = h.join();
        }
    }
}

impl Store {
    pub fn open(paths: &Paths) -> Store {
        let file = paths.store_file();
        let (flush_tx, flush_thread) = spawn_flusher(file.clone());
        // JS: seed iff the file is missing/unparseable or has no `cards` object.
        let raw: Option<Value> = std::fs::read_to_string(&file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        let parsed: Option<StoreData> = match &raw {
            Some(Value::Object(o)) if o.get("cards").map(Value::is_object).unwrap_or(false) => {
                serde_json::from_value(raw.clone().unwrap()).ok()
            }
            _ => None,
        };

        let mut store = Store { data: match parsed {
            Some(d) => d,
            None => {
                let data = StoreData {
                    version: 0,
                    board_id: default_board(),
                    custom_seq: 0,
                    cards: core::seed_cards(),
                };
                write_store_file(&file, &to_json_indent1(&data));
                data
            }
        }, file, paths: paths.clone(), flush_tx, flush_thread };

        // Legacy migration: a `skills` array in store.json → data/skills/<name>/SKILL.md.
        if let Some(skills_arr) = store.legacy_skills() {
            for s in skills_arr {
                let name = s
                    .get("id")
                    .or_else(|| s.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let description = s
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("migrated from the JSON store")
                    .to_string();
                let content = s
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                // Exists already, or invalid — leave it.
                let _ = skills::create_skill(
                    &store.paths,
                    &serde_json::json!({
                        "name": name,
                        "description": description,
                        "content": content,
                    }),
                );
            }
            store.persist_blocking();
        }

        // CamelCase handle migration: assign stable handles to cards that predate them.
        if core::ensure_handles(&mut store.data.cards) {
            store.persist_blocking();
        }

        store
    }

    fn legacy_skills(&mut self) -> Option<Vec<Value>> {
        // `skills` is not part of StoreData; detect it on the raw file once.
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&self.file).ok()?).ok()?;
        let arr = raw.get("skills")?.as_array()?;
        Some(arr.clone())
    }

    /// Synchronous persist — used only during open(), where the file must be
    /// on disk before startup logic (legacy migration) re-reads it.
    fn persist_blocking(&self) {
        write_store_file(&self.file, &to_json_indent1(&self.data));
    }

    /// Serialize in memory and hand the write to the background flusher.
    fn persist(&self) {
        let _ = self.flush_tx.send(to_json_indent1(&self.data));
    }

    /// Persist with a version bump — call after every mutation.
    pub fn save(&mut self) {
        self.data.version += 1;
        crate::log::debug(
            "store.save",
            &[
                ("version", serde_json::json!(self.data.version)),
                ("cards", serde_json::json!(self.data.cards.len())),
            ],
        );
        self.persist();
    }

    pub fn reset(&mut self) {
        self.data.board_id = default_board();
        self.data.cards = core::seed_cards();
        self.data.custom_seq = 0;
        crate::log::info("store.reset", &[]);
        self.save();
    }
}
