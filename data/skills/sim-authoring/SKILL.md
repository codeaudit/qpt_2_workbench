---
name: sim-authoring
description: Write new tutorial simulations and practice exercises for the workbench's learning system in the correct data formats. Use when the user wants a new guided scenario, a narrated walkthrough, another practice exercise, or changes to the existing tour/simulation content in qpt-data.js.
---

# Simulation & Exercise Authoring

The learning content lives in `qpt-data.js` under `QPT_DATA.learn` (`tour`, `exercises`, `simulation`). The engine drives the real board — scripts never fake state.

## Simulation step format (`learn.simulation[]`)

```js
{ board: "protocol",            // switch first if different from current
  sel: '.card[data-id="sim1"]', // spotlight target, or null (e.g. modal steps)
  text: "Narration — what is happening and WHY, with §refs.",
  run: (api) => { /* real board calls */ } }
```

API surface: `api.createSimCard()` (recreates the protagonist `sim1` at Initiation — step 1 must call it), `api.move(id, col)`, `api.edit(id, fn)`, `api.open(id, focus?)`, `api.close()`, `api.promoteTo(id, board)`.

## Storytelling rules (this is what makes the good ones good)

- **Refusals are lessons.** Deliberately script a blocked move (A9, §14, A13, A7) and narrate it — "watch the gate refuse" teaches more than a smooth pass. Blocked moves auto-open the modal; script the satisfying edit next so the pending move visibly completes.
- **Horizon is honest**: never `move` more than one column per step.
- **Numbers must check out**: if the text says S ≥ θ, compute it from the card's actual ρ/δ/γ/k first. Readers will.
- ~16 steps max; arc: entry → encounter → gate (fail) → diagnose → gate (pass) → name → closure → (promote) → home.
- Test with `index.html?sim=1&step=N` for each N you changed; snapshot/restore keeps user state safe.

## Exercise format (`learn.exercises[]`)

`{ id, board, title, refs[], goal, hint, setup: {…card fields…}, done: (c) => bool, explain }`.
`done` is a pure predicate on the practice card — grade by state, never by clicks. `setup` must put the card where the lesson starts (e.g. failing in `gate` with `pathology: null` so §14 enforcement teaches itself).

## Registry etiquette

Cite sections that exist (check qpt-spec-lookup habits) — tour and sim text carry §refs, and wrong ones erode trust fast.
