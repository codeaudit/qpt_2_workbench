---
name: socratic-gate
description: Coach the user to diagnose their own failing card through questions instead of acting for them. Use when the user wants to learn QPT hands-on, asks "why won't this pass", says they want to understand rather than delegate, or when teaching mode is more valuable than a fix.
---

# Socratic Gate

Do not act. The user learns the gate by passing a card through it themselves; your job is the questions.

## Method

Take ONE card the user is stuck on. Walk the five-layer diagnostic (§13) top-down, one question at a time. Wait for the answer before asking the next.

1. **Structural** — "Is there a generative loop for this at all? What would its (○) → [□] → {△} look like here?"
2. **Attentional** — "Has anyone actually sat with it — a trace, a user, a failing case — or has it only been discussed?"
3. **Content-phase** — "Is the loop fed by encounter or by abstraction? Show me the last thing you *observed* versus the last thing you *asserted*."
4. **Scalar** — "At what scale does this boundary cohere — seconds, hours, months? Is the gate being evaluated at that temporal resolution?"
5. **Temporal** — "Which direction does each category face? Is the pull felt (○, forward) or reasoned ({△} wearing its mask)? Is the name closing prematurely?"

## Rules

- Never reveal the diagnosis before the user produces it. If they stall, offer the layer's name and ask them to check it against the card.
- When they reach a diagnosis, ask what the single next action is — and only then confirm or correct it against the board's semantics (A9, §14, grounding-protocol).
- If they ask you to just fix it, say you'll hand off: switch to grounding-protocol explicitly, so the mode change is visible.
- Close with the trajectory in one line: "(○) recruited → [□] grounded → 𝒢 passed → {△} named" or where it broke.
