/* QPT Workbench — canonical server-side store.
 *
 * Owns the single source of truth for cards, active board, and skills, persisted
 * to data/store.json (atomic writes, version counter for sync). All mutations go
 * through qpt-core.js, so the store enforces exactly the semantics the browser does.
 */
import { readFile, writeFile, mkdir, rename } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createSkill } from "./server-skills.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// qpt-data.js / qpt-core.js are browser scripts (no ESM exports); load them by evaluation.
export async function loadCore() {
  const dataCode = await readFile(path.join(__dirname, "qpt-data.js"), "utf8");
  const QPT_DATA = new Function(dataCode + "\n;return QPT_DATA;")();
  const coreCode = await readFile(path.join(__dirname, "qpt-core.js"), "utf8");
  const moduleBox = { exports: {} };
  new Function("module", "exports", "self", "window", coreCode)(moduleBox, moduleBox.exports, {}, {});
  return { QPT_DATA, CORE: moduleBox.exports.createCore(QPT_DATA) };
}

export async function openStore() {
  const { QPT_DATA, CORE } = await loadCore();
  const file = path.join(__dirname, "data", "store.json");

  let data = null;
  try { data = JSON.parse(await readFile(file, "utf8")); } catch { /* first run */ }
  if (!data || typeof data !== "object" || !data.cards) {
    data = { version: 0, boardId: "protocol", customSeq: 0, cards: CORE.seedCards() };
    await persist();
  }

  // skills moved out of the JSON store → data/skills/<name>/SKILL.md (agentskills.io)
  if (Array.isArray(data.skills)) {
    for (const s of data.skills) {
      try {
        await createSkill({
          name: s.id || s.name,
          description: s.description || "migrated from the JSON store",
          content: s.content || "",
        });
      } catch { /* exists already, or invalid — leave it */ }
    }
    delete data.skills;
    await persist();
  }

  async function persist() {
    await mkdir(path.dirname(file), { recursive: true });
    const tmp = file + ".tmp";
    await writeFile(tmp, JSON.stringify(data, null, 1));
    await rename(tmp, file);
  }

  return {
    data,
    file,
    CORE,
    QPT_DATA,
    // persist with a version bump — call after every mutation
    async save() {
      data.version = (data.version || 0) + 1;
      await persist();
    },
    async reset() {
      data.boardId = "protocol";
      data.cards = CORE.seedCards();
      data.customSeq = 0;
      await this.save();
    },
  };
}
