---
name: gate-triage
description: Triage a whole board of failing or stalled protocol cards by verdict and zone, then route each to the right next action. Use when asked to clean up the board, fix everything in the gate, review stalled work, or decide what to do with multiple dead/deluded/competent-dead cards at once.
---

# Gate Triage

Board-level routing for the Generative Protocol. Classify first, act second — never batch-edit without a per-card route.

## Step 1 — Inventory

`set_board protocol`, then list every card with its column, verdict, and zone. Cards in `encounter`/`gate` failing the gate are the workload.

## Step 2 — Classify by verdict

| Verdict | Meaning | Route |
|---|---|---|
| Beautiful delusion | R present, G absent | Needs causal encounter, not more talk. Move to `encounter` if not there; note what contact is required. Do not advance it. |
| Competent-dead | G present, R absent | Recruitment problem — no metric edit helps. Either re-aim `source` to `recruited` with written justification, or explicitly accept it as non-living competent execution. |
| Fully dead | Neither conjunct | Check death mode (fossil / residue / imposition). If diagnosable, `promote_card` to `resolution` (Phase 0). If simply unfounded, send back through Initiation with a note. |
| At threshold | R∧G, S<θ | Zone 2. Hold; one more anchor of evidence (see grounding-protocol). Never push past on a coin flip. |

## Step 3 — Report, then act

Produce the routing table first: card id, title, verdict, zone, route. Ask before bulk mutations. Then execute one card at a time, honoring the rules:

- Forward moves are one column at a time (A13).
- A failing card leaving the gate backward needs a `pathology` layer first (§14) — set it with a one-line reason.
- Dead-structure escalations to resolution carry their diagnosis as tags automatically — verify they look right.

## Output format

```
## Gate triage — <date>
| card | verdict | zone | route |
| …    | …       | …    | …     |
Actions taken: <n> · Deferred with reason: <n>
```
