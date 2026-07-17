/* QPT Workbench — skills on disk, in the Agent Skills open format.
 *
 * Each skill is a directory data/skills/<name>/ containing SKILL.md:
 *
 *   ---
 *   name: grounding-protocol
 *   description: House rules for gate edits — when to touch rho vs theta.
 *   ---
 *   Markdown instructions…
 *
 * Spec: https://agentskills.io/specification
 *  - name: 1–64 chars, lowercase a-z 0-9 and hyphens, no leading/trailing/consecutive
 *    hyphens, must match the parent directory name
 *  - description: 1–1024 chars, non-empty
 *  - optional frontmatter (license, compatibility, metadata, allowed-tools) preserved
 */
import { readFile, writeFile, mkdir, readdir, rename, rm, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SKILLS_DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), "data", "skills");

/* ------------------------------------------------------- frontmatter io */

function unquote(v) {
  const t = String(v).trim();
  if ((t.startsWith('"') && t.endsWith('"')) || (t.startsWith("'") && t.endsWith("'"))) {
    try { return JSON.parse(t); } catch { return t.slice(1, -1); }
  }
  return t;
}

function yamlScalar(v) {
  const s = String(v).replace(/\s*\n\s*/g, " ").trim();
  return /^[a-zA-Z0-9(]/.test(s) && !/[:#][\s]/.test(s) ? s : JSON.stringify(s);
}

function dumpSkillMd({ name, description, license, compatibility, metadata, allowedTools, body }) {
  const lines = ["---", "name: " + yamlScalar(name), "description: " + yamlScalar(description)];
  if (license) lines.push("license: " + yamlScalar(license));
  if (compatibility) lines.push("compatibility: " + yamlScalar(compatibility));
  if (metadata && Object.keys(metadata).length) {
    lines.push("metadata:");
    Object.entries(metadata).forEach(([k, v]) => lines.push("  " + k + ": " + JSON.stringify(String(v))));
  }
  if (allowedTools) lines.push("allowed-tools: " + yamlScalar(allowedTools));
  lines.push("---", "");
  return lines.join("\n") + (body || "") + "\n";
}

function parseSkillMd(text) {
  const m = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/.exec(text);
  if (!m) return null;
  const fm = {};
  let metadata = null;
  m[1].split("\n").forEach((line) => {
    const nested = /^\s+([^:]+):\s*(.*)$/.exec(line);
    if (nested && metadata) { metadata[nested[1].trim()] = unquote(nested[2]); return; }
    const kv = /^([A-Za-z_-]+):\s*(.*)$/.exec(line);
    if (!kv) return;
    if (kv[1] === "metadata") { metadata = {}; fm.metadata = metadata; }
    else fm[kv[1]] = unquote(kv[2]);
  });
  return { fm, body: m[2].replace(/^\s*\n/, "") };
}

// read the CLI's config.toml (shared with server.js for model listing)
export async function readCliConfigToml() {
  const os = await import("node:os");
  const home = process.env.KIMI_CODE_HOME || path.join(os.homedir(), ".kimi-code");
  try {
    return await readFile(path.join(home, "config.toml"), "utf8");
  } catch {
    return null;
  }
}

/* ------------------------------------------------------------ validation */

function nameError(name) {
  if (!name || typeof name !== "string") return "name is required";
  if (name.length > 64) return "name must be ≤ 64 characters";
  if (!/^[a-z0-9-]+$/.test(name)) return "name: lowercase letters, numbers, and hyphens only";
  if (name.startsWith("-") || name.endsWith("-")) return "name must not start or end with a hyphen";
  if (name.includes("--")) return "name must not contain consecutive hyphens";
  return null;
}

function descError(d) {
  if (!d || !String(d).trim()) return "description is required (non-empty, ≤ 1024 chars)";
  if (String(d).length > 1024) return "description must be ≤ 1024 characters";
  return null;
}

export function slugify(s) {
  return String(s || "").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/-{2,}/g, "-").replace(/^-+|-+$/g, "");
}

/* ------------------------------------------------------------------ crud */

function skillDir(id) {
  // ids are validated slugs — never allow path traversal
  if (nameError(id)) throw new Error("invalid skill name: " + nameError(id));
  return path.join(SKILLS_DIR, id);
}

function toRecord(id, parsed, mtime) {
  const { fm, body } = parsed;
  return {
    id,
    name: fm.name || id,
    description: fm.description || "",
    content: body,
    license: fm.license || undefined,
    compatibility: fm.compatibility || undefined,
    metadata: fm.metadata || undefined,
    allowedTools: fm["allowed-tools"] || undefined,
    updated: mtime,
  };
}

export async function listSkills() {
  const out = [];
  const warnings = [];
  let entries = [];
  try {
    entries = await readdir(SKILLS_DIR, { withFileTypes: true });
  } catch { return { skills: out, warnings }; }
  for (const e of entries) {
    if (!e.isDirectory()) continue;
    const file = path.join(SKILLS_DIR, e.name, "SKILL.md");
    try {
      const text = await readFile(file, "utf8");
      const parsed = parseSkillMd(text);
      if (!parsed) { warnings.push(e.name + ": no YAML frontmatter"); continue; }
      if (parsed.fm.name && parsed.fm.name !== e.name) {
        warnings.push(e.name + ": name field " + JSON.stringify(parsed.fm.name) + " ≠ directory name (spec)");
      }
      const st = await stat(file);
      out.push(toRecord(e.name, parsed, st.mtimeMs));
    } catch { /* no SKILL.md inside — not a skill */ }
  }
  out.sort((a, b) => a.id.localeCompare(b.id));
  return { skills: out, warnings };
}

export async function readSkill(id) {
  const file = path.join(skillDir(id), "SKILL.md");
  const text = await readFile(file, "utf8");
  const parsed = parseSkillMd(text);
  if (!parsed) throw new Error("skill " + id + " has no valid frontmatter");
  const st = await stat(file);
  return toRecord(id, parsed, st.mtimeMs);
}

export async function createSkill({ name, description, content, license, compatibility, metadata, allowedTools }) {
  const id = nameError(name) == null ? name : slugify(name);
  const ne = nameError(id);
  if (ne) throw new Error(ne);
  const de = descError(description);
  if (de) throw new Error(de);
  try {
    await stat(skillDir(id));
    throw new Error("skill " + id + " already exists");
  } catch (e) {
    if (!e.code) throw e; // rethrow our own; ENOENT means free to create
  }
  const body = String(content || "");
  if (body.split("\n").length > 500) {
    // spec recommendation: keep SKILL.md under 500 lines — warn, don't block
    console.warn("skills: " + id + " body exceeds 500 lines (spec recommendation)");
  }
  await mkdir(skillDir(id), { recursive: true });
  await writeFile(path.join(SKILLS_DIR, id, "SKILL.md"),
    dumpSkillMd({ name: id, description: String(description).trim(), license, compatibility, metadata, allowedTools, body }));
  return readSkill(id);
}

export async function updateSkill(id, patch) {
  const current = await readSkill(id); // throws if missing
  const next = {
    name: patch.name != null ? String(patch.name) : current.name,
    description: patch.description != null ? String(patch.description) : current.description,
    license: patch.license !== undefined ? patch.license : current.license,
    compatibility: patch.compatibility !== undefined ? patch.compatibility : current.compatibility,
    metadata: patch.metadata !== undefined ? patch.metadata : current.metadata,
    allowedTools: patch.allowedTools !== undefined ? patch.allowedTools : current.allowedTools,
    body: patch.content != null ? String(patch.content) : current.content,
  };
  const ne = nameError(next.name);
  if (ne) throw new Error(ne);
  const de = descError(next.description);
  if (de) throw new Error(de);
  if (next.name !== id) {
    // spec: name must match the parent directory — rename it
    const target = skillDir(next.name);
    try {
      await stat(target);
      throw new Error("skill " + next.name + " already exists");
    } catch (e) {
      if (!e.code) throw e;
    }
    await rename(skillDir(id), target);
  }
  await writeFile(path.join(SKILLS_DIR, next.name, "SKILL.md"), dumpSkillMd(next));
  return readSkill(next.name);
}

export async function deleteSkill(id) {
  const dir = skillDir(id);
  await stat(dir); // throws ENOENT if missing
  await rm(dir, { recursive: true, force: true });
  return id;
}

/* ------------------------------------------- browse skills across scopes
 *
 * Kimi Code CLI discovery (docs: customization/skills): Project > User > Extra > Built-in
 *   user:    $KIMI_CODE_HOME/skills (~/.kimi-code/skills), ~/.agents/skills
 *   project: <nearest .git ancestor>/.kimi-code/skills, .agents/skills
 *   extra:   extra_skill_dirs in config.toml
 * Kimi's format also allows flat-form <name>.md files and extra frontmatter
 * (type, whenToUse, disableModelInvocation, arguments) — parsed leniently here.
 */

async function listDirSkills(dir, scope, writable) {
  const out = [];
  try {
    const entries = await readdir(dir, { withFileTypes: true });
    for (const e of entries) {
      try {
        let file = null, id = null;
        if (e.isDirectory()) {
          const candidate = path.join(dir, e.name, "SKILL.md");
          await stat(candidate);
          file = candidate;
          id = e.name;
        } else if (e.isFile() && e.name.endsWith(".md")) {
          file = path.join(dir, e.name);
          id = e.name.slice(0, -3);
        }
        if (!file) continue;
        const parsed = parseSkillMd(await readFile(file, "utf8"));
        if (!parsed) continue;
        const st = await stat(file);
        const fm = parsed.fm;
        const firstLine = (parsed.body.split("\n").find((l) => l.trim()) || "").slice(0, 240);
        out.push({
          id,
          name: fm.name || id,
          description: fm.description || firstLine,
          content: parsed.body,
          type: fm.type || undefined,
          whenToUse: fm.whenToUse || fm["when-to-use"] || fm.when_to_use || undefined,
          scope,
          writable: !!writable,
          location: file,
          updated: st.mtimeMs,
        });
      } catch { /* not a readable skill entry */ }
    }
  } catch { /* directory absent */ }
  return out;
}

async function findProjectRoot(start) {
  let dir = start;
  for (;;) {
    try {
      await stat(path.join(dir, ".git"));
      return dir;
    } catch {
      const parent = path.dirname(dir);
      if (parent === dir) return null;
      dir = parent;
    }
  }
}

async function extraSkillDirs() {
  const toml = await readCliConfigToml();
  if (!toml) return [];
  const m = /^\s*extra_skill_dirs\s*=\s*\[([^\]]*)\]/m.exec(toml);
  if (!m) return [];
  const os = await import("node:os");
  return [...m[1].matchAll(/"([^"]*)"/g)].map((x) =>
    x[1].startsWith("~") ? path.join(os.homedir(), x[1].slice(1)) : x[1]);
}

export async function listAllSkills() {
  const os = await import("node:os");
  const kimiHome = process.env.KIMI_CODE_HOME || path.join(os.homedir(), ".kimi-code");
  const projectRoot = await findProjectRoot(path.dirname(fileURLToPath(import.meta.url)));

  const scopes = [];
  const own = await listSkills();
  scopes.push({
    scope: "workbench",
    dir: SKILLS_DIR,
    writable: true,
    skills: own.skills.map((s) => ({ ...s, scope: "workbench", writable: true })),
    warnings: own.warnings,
  });

  const userDirs = [
    path.join(kimiHome, "skills"),
    path.join(os.homedir(), ".agents", "skills"),
  ];
  let userSkills = [];
  for (const d of userDirs) userSkills = userSkills.concat(await listDirSkills(d, "user", false));
  scopes.push({ scope: "user", dir: userDirs.join(" · "), writable: false, skills: userSkills, warnings: [] });

  let projectSkills = [];
  let projectDirLabel = "(no .git ancestor)";
  if (projectRoot) {
    const dirs = [path.join(projectRoot, ".kimi-code", "skills"), path.join(projectRoot, ".agents", "skills")];
    projectDirLabel = dirs.join(" · ");
    for (const d of dirs) projectSkills = projectSkills.concat(await listDirSkills(d, "project", false));
  }
  scopes.push({ scope: "project", dir: projectDirLabel, writable: false, skills: projectSkills, warnings: [] });

  let extraSkills = [];
  const extras = await extraSkillDirs().catch(() => []);
  for (const d of extras) extraSkills = extraSkills.concat(await listDirSkills(d, "extra", false));
  scopes.push({ scope: "extra", dir: extras.join(" · ") || "(none configured)", writable: false, skills: extraSkills, warnings: [] });

  // mark shadowing: Project > User > Extra — same name at higher priority wins
  const priority = { project: 0, user: 1, extra: 2, workbench: 3 };
  const seen = new Map();
  scopes.forEach((sc) => sc.skills.forEach((s) => {
    const key = s.name.toLowerCase();
    if (!seen.has(key) || priority[s.scope] < priority[seen.get(key).scope]) seen.set(key, s);
  }));
  scopes.forEach((sc) => sc.skills.forEach((s) => {
    s.shadowed = seen.get(s.name.toLowerCase()) !== s;
  }));

  return { scopes, builtin: "Built-in skills ship with the CLI itself (no files to browse)." };
}

// copy an external skill into the workbench store (spec-valid agentskills.io form)
export async function importSkill(id) {
  const { scopes } = await listAllSkills();
  let hit = null;
  scopes.forEach((sc) => sc.skills.forEach((s) => { if (s.id === id && s.scope !== "workbench") hit = hit || s; }));
  if (!hit) throw new Error("no external skill " + id);
  return createSkill({
    name: id,
    description: hit.description || "imported from " + hit.scope + " scope",
    content: hit.content || "",
  });
}

/* ------------------------------------------------- the skill generator */

export const GENERATOR_SKILL_ID = "skill-generator";

const GENERATOR_BODY = `You are a skill author. Given a short hint, you write ONE skill in the Agent Skills format (agentskills.io/specification).

OUTPUT CONTRACT — respond with ONLY a JSON object, no prose, no fences:
{"name": "…", "description": "…", "content": "…"}

RULES for name:
- 1–64 chars, lowercase letters/numbers/hyphens, no leading/trailing/consecutive hyphens.
- Short, memorable, verb- or domain-led (e.g. grounding-protocol, review-pr, jargon-audit).
- MUST NOT collide with an existing skill (the caller lists taken names).

RULES for description (the trigger line — most important field):
- 1–1024 chars. State WHAT the skill does AND WHEN to use it.
- Pack concrete keywords an agent would match on ("use when …", mentions, file types, tasks).

RULES for content (Markdown body):
- Step-by-step procedure first, then examples (input → output), then edge cases.
- Prefer checklists and numbered steps over essays.
- Under ~120 lines. If reference material is needed, say so and name a references/ file.
- No YAML frontmatter in content — the caller writes the frontmatter.

QUALITY BAR: the generated skill must be usable verbatim by an agent that has never seen the hint. No placeholders like "TODO" or "fill this in".`;

export async function ensureSkillGenerator() {
  try {
    await readSkill(GENERATOR_SKILL_ID);
  } catch {
    await createSkill({
      name: GENERATOR_SKILL_ID,
      description: "Generate a new Agent Skills skill from a short hint. Use when the user wants a new skill drafted — outputs spec-valid name, trigger description, and Markdown instructions.",
      content: GENERATOR_BODY,
    });
  }
}
