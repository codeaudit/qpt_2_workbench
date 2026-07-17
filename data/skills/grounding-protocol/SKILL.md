---
name: grounding-protocol
description: Fix a protocol card that fails the Quality Gate (S < θ, or a missing R/G conjunct). Use when asked to make a card living, get it past the gate, raise its anchoring score, or when a card is stuck in Gate Evaluation and cannot advance to Articulation.
---

# Grounding Protocol

How to make a failing protocol card living — the honest way, lever by lever. Never fake the conjuncts.

## Inputs

Run `evaluate_card` (or read the card's compact fields) and record: verdict, `S = rho − delta − gamma·ln(k)` vs `theta`, trajectory (`source`, `target`), zone (Z1 drift / Z2 transition / Z3 anchored).

## Lever order — try each before moving to the next

1. **Trajectory to `[□]`-grounded — only with an encounter note.**
   Changing `target` to `grounded` without evidence is exactly the "beautiful delusion" the gate exists to catch. Before this edit, write what the causal encounter was (a trace, a profile, a failing case, a user quote) into the card's `note` via `edit_card`. If no encounter happened, the correct action is NOT an edit — it is moving the card to `encounter` and telling the user what contact is needed.
2. **Raise `rho` (effective support).** More self-consistency: more anchors, better consensus. Increase toward (not past) ~0.95; values of 1.0 are suspect — treat as δ hiding.
3. **Reduce `delta` (representational mismatch).** Stabilize the representation: cleaner spec, less perturbation. Each 0.05 reduction is a real improvement; do not zero it without justification.
4. **Reduce `k` (anchor count).** Only when anchors are genuinely redundant — each one costs `gamma·ln(k)`.
5. **Adjust `theta` — last, and only if the threshold is miscalibrated for the task.** Document why in the note. Reaching for θ first is gaming the gate (PMG in miniature).

## Special cases

- **`source: initiated` (R absent).** No metric edit can make the card living — the gate's R conjunct is not a number. Either re-aim `source` to `recruited` with a written justification of what recruited attention, or accept the card as competent-dead and say so to the user.
- **Zone 2 (`S ≈ θ`).** Do not force it. Hold at the gate: gather one more anchor of evidence (nudge ρ up or δ down slightly) and re-evaluate.
- **After passing.** Move forward exactly one column (A13). If the card fails and must return, a pathology layer is required first (§14) — set it via `edit_card pathology=…` with a one-line justification.

## Verify

Re-run `evaluate_card`. Report the new S vs θ and which lever did the work — not just "fixed".
