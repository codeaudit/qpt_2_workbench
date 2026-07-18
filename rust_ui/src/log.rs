//! Port of server-log.js — leveled logging to console + data/server.log
//! (rotated to .1 at ~1 MB). Never logs secrets or prompt bodies.

use serde_json::Value;
use std::sync::{Mutex, OnceLock};

const ROTATE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug = 10,
    Info = 20,
    Warn = 30,
    Error = 40,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

struct Logger {
    threshold: Level,
    file: std::path::PathBuf,
    /// kept open in append mode between lines (no per-line open/close)
    handle: Option<std::fs::File>,
    /// bytes written so far — the rotation trigger, no per-line stat
    written: u64,
}

impl Logger {
    /// Append one formatted line, rotating to `.1` once past 1 MB. The check
    /// happens BEFORE the write and a failed rotate drops the line, exactly
    /// like the old per-line open/close version.
    fn append(&mut self, line: &str) -> std::io::Result<()> {
        use std::io::Write;
        if self.handle.is_none() {
            if let Some(dir) = self.file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let f = std::fs::OpenOptions::new().create(true).append(true).open(&self.file)?;
            self.written = f.metadata().map(|m| m.len()).unwrap_or(0);
            self.handle = Some(f);
        }
        if self.written > ROTATE_BYTES {
            self.handle = None; // close before the rename
            std::fs::rename(&self.file, self.file.with_extension("log.1"))?;
            let f = std::fs::OpenOptions::new().create(true).append(true).open(&self.file)?;
            self.written = 0;
            self.handle = Some(f);
        }
        let Some(h) = self.handle.as_mut() else { return Ok(()) };
        writeln!(h, "{}", line)?;
        self.written += line.len() as u64 + 1;
        Ok(())
    }
}

static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

fn logger() -> &'static Mutex<Logger> {
    LOGGER.get_or_init(|| {
        let threshold = match std::env::var("LOG_LEVEL").as_deref() {
            Ok("debug") => Level::Debug,
            Ok("warn") => Level::Warn,
            Ok("error") => Level::Error,
            _ => Level::Info,
        };
        let file = crate::Paths::cwd().log_file();
        Mutex::new(Logger { threshold, file, handle: None, written: 0 })
    })
}

fn fmt_fields(fields: &[(String, Value)]) -> String {
    fields
        .iter()
        .filter(|(_, v)| !v.is_null() && v.as_str() != Some(""))
        .map(|(k, v)| {
            let s = match v {
                Value::String(s) => s.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            if s.chars().any(char::is_whitespace) {
                format!("{}={}", k, serde_json::to_string(&s).unwrap_or_default())
            } else {
                format!("{}={}", k, s)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn write(level: Level, event: &str, fields: &[(String, Value)]) {
    let Ok(mut logger) = logger().lock() else { return };
    if level < logger.threshold {
        return;
    }
    let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let f = fmt_fields(fields);
    let line = format!("[{}] {:<5} {}{}", ts, level.name(), event, if f.is_empty() { String::new() } else { format!(" {}", f) });
    match level {
        Level::Error => eprintln!("{}", line),
        _ => println!("{}", line),
    }
    // Logging must never break the server — every fs error is swallowed.
    let _ = logger.append(&line);
}

fn fields(pairs: &[(&str, Value)]) -> Vec<(String, Value)> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

/// Cheap gate for the public fns: no field building for discarded lines.
fn enabled(level: Level) -> bool {
    logger().lock().map(|l| level >= l.threshold).unwrap_or(false)
}

pub fn debug(event: &str, pairs: &[(&str, Value)]) {
    if enabled(Level::Debug) {
        write(Level::Debug, event, &fields(pairs));
    }
}
pub fn info(event: &str, pairs: &[(&str, Value)]) {
    if enabled(Level::Info) {
        write(Level::Info, event, &fields(pairs));
    }
}
pub fn warn(event: &str, pairs: &[(&str, Value)]) {
    if enabled(Level::Warn) {
        write(Level::Warn, event, &fields(pairs));
    }
}
pub fn error(event: &str, pairs: &[(&str, Value)]) {
    if enabled(Level::Error) {
        write(Level::Error, event, &fields(pairs));
    }
}
