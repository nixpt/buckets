# RULES — standing discipline for anyone working this backlog

Standing developer discipline for anyone working this backlog. These are not suggestions — every agent touching `.jagent/planning/`'s backlog follows them.

## 1. Verify before you fix

**A ticket's `Backlog` status is a claim, not a fact.** Before spending any effort on a ticket:

1. Re-run its own `## Reproduction` section verbatim, against current `master` (pull first, run against unmodified master).
2. If it no longer reproduces: update the ticket's `Status` to `Done` with a one-paragraph `## Resolution` section (what you ran, what it printed, and what commit likely fixed it if known) and mark the corresponding `TASKS.md` line `[x]`. Do not silently delete the ticket.
3. If it does reproduce: proceed to fix it. If the fix turns out different in shape than the ticket's `## Technical approach` guessed, that's fine.

This applies recursively: if you find a NEW bug while working an existing ticket, don't fold it silently into the same commit — file it as its own `BUCKETS-N` ticket so it gets its own verify-before-fix cycle later.

## 2. One worktree + branch per milestone (or per ticket, whichever is smaller)

**Every unit of work gets its own worktree and its own branch.** Never work directly on `master`.

```bash
# per ticket:
git worktree add /home/nixp/worktrees/antigravity/BUCKETS-N -b agent/<name>/BUCKETS-N origin/master

# per milestone (several related tickets):
git worktree add /home/nixp/worktrees/antigravity/M1-correctness -b agent/<name>/M1-CORRECTNESS origin/master
```

Why per-milestone and not one giant branch for the whole backlog: a milestone-sized branch is small enough for foreman/captain to review and merge incrementally, and a bad turn doesn't put other already-good work at risk of rollback.

## 3. Commit + push at every milestone/phase boundary — don't batch to the end

**Push when a milestone (or ticket, if working ticket-by-ticket) is done — not when the whole backlog is done.** Concretely:

1. Finish the milestone's work. Run its own verification (see the ticket's success criteria, plus at minimum `cargo check` and `cargo test`).
2. Commit with a message that names the ticket(s) closed.
3. Push the branch: `git push -u origin agent/<name>/<MILESTONE-OR-TICKET>`.
4. Post to the bridge/logs naming what shipped, what's verified, and what's next.
5. **Before starting the next milestone**, pull the latest `master` into a fresh worktree+branch.

## 4. Update `.jagent/planning/` as you go, not as an afterthought

- Mark `TASKS.md` checkboxes `[x]` the moment something is verifiably done.
- Update the closed ticket's own `Status` field and add a `## Resolution` section — future agents read the ticket file directly, they don't re-derive status from git log.
- If you file a new ticket (a bug found while working something else), use the existing template (`.jagent/planning/templates/ticket.md`) and the next available `BUCKETS-N` number.

## Cross-references

- `.jagent/planning/TASKS.md` — the current backlog these rules apply to
- `.jagent/planning/ROADMAP.md` — the milestone sequence
- `.jagent/planning/tickets/` — one file per `BUCKETS-N`
