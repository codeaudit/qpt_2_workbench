---
name: board-health-report
description: Produce a periodic health audit of all boards — verdict and zone distribution, stuck cards, weight tables, trace anomalies — in a fixed comparable format. Use when asked for a status report, board review, weekly summary, "how is the system doing", or before deciding where to spend effort next.
---

# Board Health Report

A repeatable audit. Always the same sections so reports compare over time. Report first, act only with approval.

## Gathering

For each board (`protocol`, `dialectic`, `resolution`), list cards with column, verdict (protocol), Γ/w (dialectic), tags (resolution), and trace length.

## Report format

```markdown
## Board health — <date>

### Distribution (protocol)
- Living n · threshold n · delusion n · competent-dead n · dead n
- Zones: Z1 n · Z2 n · Z3 n

### Stuck cards
- In gate longest: <id — title, since when known, verdict>
- In same column longest per board: <id — column>

### Diagnoses
- Pathology layers: structural n · attentional n · content n · scalar n · temporal n
- Death modes: fossil n · residue n · imposition n

### Dialectic
- Positions by phase: explore n · integrate n · consolidate n · synthesize n
- Weights: u₁ w… · u₂ w… (flag any w > 0.5 — single-voice risk)

### Anomalies
- Cards with empty traces (never moved): <ids>
- Promotions this period: <count, paths used>

### Recommended actions (≤ 3)
1. … 2. … 3. …
```

## Rules

- Every claim must be checkable from the state — cite card ids, not impressions.
- Recommended actions reference a route, not a mood: ground this card (grounding-protocol), escalate that fossil (promotion-paths), advance this position (dialectic-moderator).
- If nothing is anomalous, say so plainly. A healthy report is a result, not a failure of effort.
