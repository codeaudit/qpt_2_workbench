/* QPT Workbench — leveled server logging.
 *
 * Two sinks:
 *   console  human-readable lines:  [time] LEVEL event k=v k=v
 *   file     same lines appended to data/server.log (rotated to .1 at ~1 MB)
 *
 * LOG_LEVEL env: debug | info (default) | warn | error.
 * Never logs secrets (API keys, tokens) or prompt bodies — sizes and ids only.
 */
import { appendFile, stat, rename, mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const LOG_DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), "data");
const LOG_FILE = path.join(LOG_DIR, "server.log");
const ROTATE_BYTES = 1024 * 1024;

const LEVELS = { debug: 10, info: 20, warn: 30, error: 40 };
const THRESHOLD = LEVELS[process.env.LOG_LEVEL || "info"] || 20;

function fmtFields(fields) {
  return Object.entries(fields || {})
    .filter(([, v]) => v !== undefined && v !== null && v !== "")
    .map(([k, v]) => {
      const s = typeof v === "string" ? v : JSON.stringify(v);
      return k + "=" + (/\s/.test(s) ? JSON.stringify(s) : s);
    })
    .join(" ");
}

function line(level, event, fields) {
  const f = fmtFields(fields);
  return "[" + new Date().toISOString() + "] " + level.toUpperCase().padEnd(5) + " " + event + (f ? " " + f : "");
}

async function rotateIfNeeded() {
  try {
    const st = await stat(LOG_FILE);
    if (st.size > ROTATE_BYTES) await rename(LOG_FILE, LOG_FILE + ".1");
  } catch { /* no file yet */ }
}

async function write(level, event, fields) {
  if (LEVELS[level] < THRESHOLD) return;
  const l = line(level, event, fields);
  if (level === "error") console.error(l);
  else if (level === "warn") console.warn(l);
  else console.log(l);
  try {
    await mkdir(LOG_DIR, { recursive: true });
    await rotateIfNeeded();
    await appendFile(LOG_FILE, l + "\n");
  } catch { /* logging must never break the server */ }
}

export const log = {
  debug: (event, fields) => write("debug", event, fields),
  info: (event, fields) => write("info", event, fields),
  warn: (event, fields) => write("warn", event, fields),
  error: (event, fields) => write("error", event, fields),
};
