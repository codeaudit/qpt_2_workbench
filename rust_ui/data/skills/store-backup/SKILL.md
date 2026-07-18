---
name: store-backup
description: Snapshot and restore the workbench's canonical state (data/store.json and data/skills/) safely before batch operations, migrations, or risky agent runs. Use before bulk edits, board resets, skill deletions, or whenever the user asks for a backup, checkpoint, or rollback.
---

# Store Backup & Restore

The canonical state is small enough to snapshot fully: `data/store.json` (cards, board, version) and `data/skills/` (every SKILL.md). Do it before anything that touches many cards at once.

## Backup (before batch work)

```bash
stamp=$(date +%Y%m%d-%H%M%S)
mkdir -p data/backups/$stamp
cp data/store.json data/backups/$stamp/store.json
cp -R data/skills data/backups/$stamp/skills
```

Verify the copy: card count matches (`node cli.js "cards" | wc -l` vs the backup's), and the skills directory listing matches. State the backup path in your reply so the user can find it.

## Restore (rollback)

```bash
cp data/backups/<stamp>/store.json data/store.json
rm -rf data/skills && cp -R data/backups/<stamp>/skills data/skills
```

Then restart the server (the store loads at startup) and verify: card count, one known card's column, one known skill listing. The browser re-hydrates from the store on its next poll.

## Rules

- Back up before: `reset`, more than ~5 card mutations in one plan, skill deletions, any hand-edit of `store.json`.
- Keep the last 5 backups; older ones are dead structure (§11) — dissolve them, one at a time, telling the user.
- Never back up over an existing backup directory; timestamps must be unique.
- Backups are not version control: if the user needs history, suggest `git` on `data/` instead of more tarballs.
