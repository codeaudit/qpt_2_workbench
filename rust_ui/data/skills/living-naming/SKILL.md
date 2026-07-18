---
name: living-naming
description: Audit or rewrite card titles so they are living names per §12 — verb-led, outcome-stating, opening new diagnostics. Use when asked to rename cards, fix jargon titles, review naming quality, or when titles look like acronyms, version references, or mechanism-speak (e.g. "Q3 dashboard", "bridge sync", "QPT migration").
---

# Living Naming (§12)

A name lives by its trajectory, not its wording: `(○)-recruited` at the source and `[□]`-grounded at the target. Jargon signals membership and enables no contact.

## Jargon tells

- Acronyms and internal codenames ("QPT", "HF", "the Phoenix thing")
- Version or section references instead of content ("r2.7 rollout", "§4.2 fix")
- Mechanism-speak about plumbing ("bridge", "shim", "wire up", "sync layer")
- Nouns of bureaucracy ("initiative", "program", "workstream", "Q3 dashboard")

## Living-name shape

- **Verb-led**: states what changes, not what category it belongs to
- **Observable outcome**: a newcomer can tell when it is done
- **Opens diagnostics**: reading it suggests the next question to ask
- 3–10 words; no punctuation gymnastics

## Examples

| Jargon | Living name |
|---|---|
| Q3 dashboard redesign | Show each on-call what their alerts cost in sleep |
| session → encounter rename | Rename `session` to what actually happened in it |
| Auth middleware cleanup | Delete the middleware that still checks pre-2023 tokens |
| Metrics initiative | Count how often deploy pain reaches a dashboard instead of a person |

## Procedure

1. List candidate titles (whole board, or the cards the user points at).
2. For each: name the tell, propose a living rewrite, and note which naming quadrant it lands in (living name / poetic capture / technical term / jargon — the last two are acceptable only deliberately).
3. Rename with `edit_card title=…` only after the user confirms the set — names are meaning, and meaning is a gate decision, not a batch job.
4. If the card sits upstream of the gate, note that any naming now is organizational (§12) — the real name arrives after passage.
