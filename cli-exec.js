/* QPT Workbench — command language over the store.
 *
 * One executor shared by the HTTP endpoint (POST /api/cli), the standalone bin
 * (node cli.js "..."), and available to the agent. Mutations run through
 * qpt-core.js — the same enforcement as every other surface. Skills live on
 * disk in the Agent Skills format (data/skills/<name>/SKILL.md).
 *
 *   help · state · boards · board <id> · cards [--board b] · card <id>
 *   create --board b --title "…" [--source s] [--target t] [--kind k] [--reliability n] [--tags a,b] [--note "…"]
 *   move <id> <column> · edit <id> key=value… · evaluate <id> · promote <id> --to <board> · reset
 *   skills · skill show <id> · skill create <id> --description "…" [--content "…"]
 *   skill edit <id> [name=…] [description=…] [content=…] · skill delete <id>
 */

import { listSkills, readSkill, createSkill, updateSkill, deleteSkill } from "./server-skills.js";

export function tokenize(s) {
  const out = [];
  // key="quoted value" | key='quoted value' | key=value | "quoted" | 'quoted' | bare
  const re = /(\S+?=)(?:"([^"]*)"|'([^']*)'|(\S+))|"([^"]*)"|'([^']*)'|(\S+)/g;
  let m;
  while ((m = re.exec(s))) {
    if (m[1] !== undefined) out.push(m[1] + (m[2] ?? m[3] ?? m[4] ?? ""));
    else out.push(m[5] ?? m[6] ?? m[7]);
  }
  return out;
}

const HELP = `qpt workbench CLI
  state                              compact store snapshot (JSON)
  boards                             boards and their column flows
  board <id>                         switch the active board (protocol|dialectic|resolution)
  cards [--board <id>]               list cards on a board
  card <id>                          full card JSON
  create --board <b> --title "…"     new card at the board's entry column
       [--source recruited|initiated] [--target grounded|terminated]
       [--kind position|note] [--reliability 0..1] [--tags a,b] [--note "…"]
  move <id> <column>                 move one column (A13: no forward skips)
  edit <id> key=value …              domain-clamped edit (rho delta gamma k theta
                                     source target scale pathology death reliability genesis title note sign agent tags)
  evaluate <id>                      gate verdict with S vs θ
  promote <id> --to <board>          cross-board promotion (paths per §15/§7.4/§11/§29)
  reset                              reseed the store from the specification
  skills                             list skills (data/skills/<name>/SKILL.md)
  skill show <id>                    print a skill
  skill create <id> --description "…" [--content "…"]
  skill edit <id> [name=…] [description=…] [content=…]
  skill delete <id>`;

function findSkill(skills, idOrName) {
  return skills.find((s) => s.id === idOrName) || skills.find((s) => s.name === idOrName);
}

export async function execCommand(line, store) {
  const t = tokenize(line);
  const ok = (output, changed = false) => ({ ok: true, output, changed });
  const bad = (output) => ({ ok: false, output, changed: false });
  if (!t.length) return bad("empty command — try `help`");

  const [cmd, ...rest] = t;
  const pos = [];
  const flags = {};
  for (let i = 0; i < rest.length; i++) {
    if (rest[i].startsWith("--")) {
      const k = rest[i].slice(2);
      flags[k] = rest[i + 1] != null && !rest[i + 1].startsWith("--") ? rest[++i] : true;
    } else pos.push(rest[i]);
  }

  const CORE = store.CORE;
  const data = store.data;

  async function finish(r) {
    if (r.ok) { await store.save(); return ok(r.message, true); }
    return bad(r.message);
  }

  switch (cmd) {
    case "help":
      return ok(HELP);

    case "state":
      return ok(JSON.stringify(CORE.compactState(data), null, 1));

    case "boards":
      return ok(CORE.boards.map((b) => b.id + ":  " + b.columns.map((c) => c.id).join(" → ")).join("\n"));

    case "board": {
      if (!CORE.boards.some((b) => b.id === pos[0])) return bad("unknown board " + (pos[0] || "(missing)"));
      data.boardId = pos[0];
      await store.save();
      return ok("board → " + pos[0], true);
    }

    case "cards": {
      const boardId = flags.board || data.boardId;
      const list = CORE.cardsOn(data, boardId);
      if (!list.length) return ok("(no cards on " + boardId + ")");
      return ok(list.map((c) => {
        const ev = CORE.evaluate(c);
        return c.id + " @ " + c.column + " — " + c.title + (ev ? "  [" + ev.key + "]" : "");
      }).join("\n"));
    }

    case "card": {
      const c = data.cards[pos[0]];
      if (!c) return bad("no card " + (pos[0] || "(missing id)"));
      return ok(JSON.stringify(c, null, 1));
    }

    case "create":
      return finish(CORE.applyAction(data, {
        action: "create_card",
        board: flags.board || data.boardId,
        title: flags.title,
        source: flags.source,
        target: flags.target,
        kind: flags.kind,
        reliability: flags.reliability != null ? parseFloat(flags.reliability) : undefined,
        tags: flags.tags ? String(flags.tags).split(",").map((x) => x.trim()).filter(Boolean) : undefined,
        note: flags.note,
        via: "the CLI",
      }));

    case "move":
      if (!pos[0] || !pos[1]) return bad("usage: move <id> <column>");
      return finish(CORE.applyAction(data, { action: "move_card", id: pos[0], column: pos[1] }));

    case "edit": {
      if (!pos[0]) return bad("usage: edit <id> key=value …");
      const patch = {};
      for (const kv of pos.slice(1)) {
        const eq = kv.indexOf("=");
        if (eq < 0) return bad("edit: expected key=value, got " + JSON.stringify(kv));
        let v = kv.slice(eq + 1);
        if (v === "null") v = null;
        else if (v === "true") v = true;
        else if (v === "false") v = false;
        else if (v !== "" && !isNaN(+v)) v = +v;
        else if (v.indexOf(",") >= 0) v = v.split(",").map((x) => x.trim()).filter(Boolean);
        patch[kv.slice(0, eq)] = v;
      }
      return finish(CORE.applyAction(data, { action: "edit_card", id: pos[0], patch }));
    }

    case "evaluate": {
      const r = CORE.applyAction(data, { action: "evaluate_card", id: pos[0] });
      return r.ok ? ok(r.message) : bad(r.message);
    }

    case "promote":
      if (!pos[0] || !flags.to) return bad("usage: promote <id> --to <board>");
      return finish(CORE.applyAction(data, { action: "promote_card", id: pos[0], board: flags.to }));

    case "reset":
      await store.reset();
      return ok("store reseeded from the specification", true);

    case "skills": {
      const { skills, warnings } = await listSkills();
      const lines = skills.map((s) => s.id + " — " + s.description.slice(0, 80) + (s.description.length > 80 ? "…" : ""));
      warnings.forEach((w) => lines.push("⚠ " + w));
      return ok(lines.length ? lines.join("\n") : "(no skills — data/skills/ is empty)");
    }

    case "skill": {
      const sub = pos[0];
      try {
        if (sub === "show") {
          const { skills } = await listSkills();
          const hit = findSkill(skills, pos[1]);
          if (!hit) return bad("no skill " + (pos[1] || "(missing id)"));
          const s = await readSkill(hit.id);
          return ok("# " + s.name + " (" + s.id + ")\n" + s.description + "\n\n" + s.content);
        }
        if (sub === "create") {
          if (!pos[1]) return bad("usage: skill create <id> --description \"…\" [--content \"…\"]");
          const s = await createSkill({
            name: pos[1],
            description: flags.description || "",
            content: flags.content || "",
          });
          return ok("skill created: data/skills/" + s.id + "/SKILL.md", true);
        }
        if (sub === "edit") {
          const { skills } = await listSkills();
          const hit = findSkill(skills, pos[1]);
          if (!hit) return bad("no skill " + (pos[1] || "(missing id)"));
          const patch = {};
          const allowed = ["name", "description", "content"];
          for (const kv of pos.slice(2)) {
            const eq = kv.indexOf("=");
            if (eq < 0) return bad("skill edit: expected key=value, got " + JSON.stringify(kv));
            const k = kv.slice(0, eq);
            if (!allowed.includes(k)) return bad("skill edit: key must be " + allowed.join("|"));
            patch[k] = kv.slice(eq + 1);
          }
          if (!Object.keys(patch).length) return bad("skill edit: nothing to change");
          const s = await updateSkill(hit.id, patch);
          return ok("skill updated: " + s.id, true);
        }
        if (sub === "delete") {
          const { skills } = await listSkills();
          const hit = findSkill(skills, pos[1]);
          if (!hit) return bad("no skill " + (pos[1] || "(missing id)"));
          await deleteSkill(hit.id);
          return ok("skill deleted: " + hit.id, true);
        }
        return bad("skill: expected show|create|edit|delete");
      } catch (e) {
        return bad(String((e && e.message) || e));
      }
    }

    default:
      return bad("unknown command " + JSON.stringify(cmd) + " — try `help`");
  }
}
