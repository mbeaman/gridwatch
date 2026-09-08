# Session journal

> One entry per working session, newest first. This is the only document that records **what a session was like** — what was tried, what the reviews caught, what turned out to be wrong, and what is owed to Matt when it ends. The others answer different questions: `DECISIONS.md` says *why a choice was made*, `CHANGELOG.md` says *what shipped*, `ROADMAP.md` says *what an arc promised*, `PLAN.md` says *where the project stands right now*. A journal entry is allowed to record a dead end, a wrong assumption or a near miss, because those are the things no other file has a place for.
>
> **Write the entry before the session ends**, while the work is still in context, and commit it with the session's last commit. Lead with what changed for Matt and why it matters — not an inventory of edits (Matt, 2026-09-07). Keep it to what a reader six months from now would need.

---

## 2026-09-07 — arc 13: the config means what it says

**Models:** Fable 5.1 for the seam design (delegated — the session had switched to Opus mid-flight and D36 reserves seams for Fable); Opus 5 for the implementation and the four review lenses. **Shipped:** arc 13, 10 commits, `1ae6294 → a9572f1`. **Nothing tagged.**

### What changed for Matt

A misspelled source option already told him. Now a *wrongly typed* one does too: `refresh_ms = "1500"` with quotes used to be discarded in silence and run on the default, and now it fails `config check` with a sentence naming what was found, what was expected and what stands. And `[store] history` is live, so asking for an hour of history gets an hour instead of ten minutes.

### The finding that changed the arc's shape

**The escalation was three keys short.** The backlog said `[store]` was the last silent section in `config.toml`. The design pass checked, and found `confirm_kill` read by nothing, `[record]` warning "arrives in arc 2" eleven arcs later, and — the interesting one — **`[perf] phase_ms` naming a mechanism that was never built**. The spec and a performance row both credited a 250 ms phase grid for the wake-up budget; deadlines actually align to each source's own cadence, and the budget is met anyway. A number can be right for a reason that does not exist.

So the arc stopped being "two escalations" and became a rule — every key the config accepts is read, or it says it is not — with an audit test that fails when a key is added without a consumer. That test is the durable part.

**`max_mb` was retired rather than built.** The spec claimed a 32 MB store cap in two places and no byte accounting existed. Building the accounting is easy; deciding what to *do* at the limit is not. Shrinking retention makes a chart's time axis depend on how many sensors the machine has. Refusing samples discards the newest data. Neither can be reported by `config check`, because it depends on what the machine publishes at runtime. A cap that quietly changes what you see is the defect the arc exists to end, so it is a measurement now (`Store::footprint()`), not a policy.

**One number made the risky half safe.** Ten minutes at four points a second is 2 400 points — exactly the `max_len` that had been hard-coded. So `history` going live is field-identical at the shipped default, and no snapshot, replay or P18 number moved. Without that coincidence the retention change would have been a gamble against the determinism tests.

### What the review caught

**A crash, in the arc's own new code.** `F12` on a terminal seven to ten rows tall took the whole app down, and the panic went only to the log — a blank terminal and exit 101. The HUD clamped its box but not its text rows; arc 13's own store line pushed the effects line one row down, which extended a pre-existing 7–9 row bug to 10. Only the user-path lens could find it: it needs a real terminal, a real keypress and a resize. A unit test now draws the HUD at every height from 0 to 14.

**A behaviour change wider than the decision described.** D63 said `refresh_ms = 0` was "discarded without a word". True of cpu and gpu — but pins, sensors, mpris and net clamped it *up* and used it, and net's three other interval keys did so without even a log line. So nine keys across five sources changed what they resolve to, where the decision's own trap licensed two. **The behaviour stands and the sentence was corrected**: zero meaning "the default" in one source and "as fast as allowed" in another was an accident of two different fallback idioms, documented nowhere. What should have carried that decision is tests, and nothing pinned any of the nine — which is why it shipped silently. They are pinned now.

**And the brief pointed at a table that does not exist.** It told the implementer the per-key contract was "the table in D63" and that table had been trimmed when the file was written. He reconstructed it from the seven parsers — exactly the work the pointer was meant to save, and exactly the drift the design had refused to create. The fix is not to write the table: E1 chose "the reader is the declaration" *because* a second copy drifts and nothing can test the drift, so the brief points at the code now.

### A process note worth keeping

**Two sessions in a row reverted their own uncommitted work with git.** The implementation agent ran `git checkout --` on a file mid-build and lost the loader work; then, reviewing it, this session ran `git stash` on a file and lost the crash fix and its test. Both were recovered. `CLAUDE.md`'s read-only guard covers *review* agents because a review agent once ran `git restore` — the same footgun is live for implementers and for the main session, and the guard does not reach either.

### Owed to Matt after this session

- **P17 at the `1h` ceiling.** A 60-minute release run is the heavy job `MACHINE.md` forbids an agent beside a game. `PERFORMANCE.md` says so plainly and records two short *debug* runs instead, labelled as such. Also owed: the 20-minute default-history run confirming the store stays under 6 MB.
- **Everything already owed**: every tag from `v0.1.0`, the version bump, his real Ptyxis size, the game fixture, the Ptyxis and i2c rows.

### Where to pick up

`BACKLOG.md`'s `Next up` is the `disk` component, then what still leaves a wide terminal blank, then focus events under tmux and ssh. Four small things this arc's review recorded rather than fixed are backlogged together, the sharpest being that the `[store] history` **load** error is the one message whose meaning sits in its middle, so an 80-column toast elides the key and the filename.

---

## 2026-09-06 → 07 — the wide-terminal bug, and the source-option seams

**Models:** Fable 5.1 throughout (decisions, briefs, review judging); Opus 5 for both implementations and the six review lenses. **Shipped:** arcs 12 and 11, 28 commits, `8547c6d → 95977af`, CI green on both pushes. **Nothing tagged** — as always, that is Matt's.

### What changed for Matt

gridwatch fills a wide terminal now. That was his report ("appears broken if the console resolution is too wide") and it is fixed, confirmed in a real terminal rather than a buffer. And a misspelled source option in `config.toml` finally says so instead of being ignored forever.

### The thing worth remembering

**Neither arc was really about the bug it was named for.**

Arc 12 looked like three components hard-coding a drawing size. The actual problem was that nothing in the project *stopped* a fourth. So the fix was a rule in `ARCHITECTURE.md` §4.6 — a drawing inside a `Fill` band scales with its rect, and a constant is only ever a floor — plus `assert_grows_with_area` to enforce it per tier. That test then caught four more instances, **two of which the arc itself had just shipped**: a pins sparkline that capped its own buckets, and a GPU power trace that was fifty columns wide on any terminal. Without the test those ship silently and Matt rediscovers them on the next resize.

The lesson that generalises: **a growth test that counts cells per *tier* misses a capped drawing that is a small share of that tier's ink.** The sweep passed pins and gpu while both were still broken; the ux and user-path lenses found them by looking at rendered frames. Cell-count oracles need a per-drawing companion, or a human looking at the screen.

Arc 11 was the same shape one level down: a config that could be quietly wrong. The seams (`SourceDef.options`, `KeyMeta.labels`) had landed on 2026-09-05; this session built the breadth and reviewed it.

### What the reviews caught that the builds did not

Six adversarial lenses ran across the two arcs (correctness, spec-drift, ux-theme, user-path ×2). The pattern held from earlier arcs: **the implementation session's own report was accurate about what it built and blind to what it had broken.**

- **Arc 12** shipped four fresh instances of the defect it existed to fix (above), plus a zoomed GPU tile that showed *fewer* table rows than the grid below 30 rows — a plain "a third of the body" rule inverts under a short terminal.
- **Arc 11** shipped a reload that called a plugin's `[sources.<id>]` table "no such source in this build" while `config check` on the same file called it fine. Cause: the re-check read the plugin ids that had *started*, and `[[plugins]]` is restart-only, so a plugin added in the very save being checked can never be in that set. This is the first-plugin flow — the most likely time anyone meets it.
- **Toast truncation had a shape assumption.** An earlier review chose "keep the tail" because a parse error leads with `file:line:col` and ends with the reason. Arc 11's messages are the opposite shape — they lead with the source and the key. At 80 columns the network warning showed *neither*, just a bare list of option names. Toasts elide the middle now, which serves both shapes.
- **`config check` claimed to have read a value it had not.** `refresh_ms = "fast"` was echoed back as though accepted while the source silently kept its default. The report now says it checks option *names*; the real fix is a seam and is in `BACKLOG.md`.

### Two things that were decided rather than built

**The four §9 option-name collisions stay, and the naming call is Matt's** (D61 review amendment R6). `[sources.audio] fps` vs the audio and winamp tiles' `fps`; `[sources.sensors] chips`; `[sources.mpris] art`. All real — the name is valid on both sides, so no check can catch a person who writes one in the wrong file. Not renamed because the harm is the mildest kind (in all four cases the wrong-file write produces roughly the intended *effect*, differing in cost rather than outcome), because `[sources.audio] fps` is D35 decision 6 which Matt named himself, and because the churn would land on D55 and `PERFORMANCE.md` rows that must keep their original words. §9 records the exemptions; the test pins them in both directions. Candidates if he wants them gone: `analysis_fps`, `sample`, `fetch_art`.

*A near miss worth recording:* the first instinct was to rename all three on a consistency argument — "this arc exists to end silent wrongness, so leave no ambiguity". A second read reversed it, on the strength of the severity analysis that was already in hand and had been argued past. **The consistency argument was aesthetic; the severity analysis was evidence.**

**The rename was not the only overreach caught.** A ticked roadmap box claimed "a replay determinism test crosses a sweep boundary that evicts". It does not: every journal fixture spans about 62 s of store time against a 600 s retention, so a replay evicts nothing. The store-level determinism test does cross an evicting sweep; the *journal* one cannot without a fixture longer than ten minutes, which does not exist. A falsely ticked box is worse than an unticked one, because the next session reads the roadmap to decide what is left.

### Owed to Matt after this session

- **His real Ptyxis window size.** `MATT_TERMINAL` in `crates/cli/tests/smoke.rs` is a `None` slot waiting for it, and `MACHINE.md` still says unknown. The moment it is filled in, the wide-terminal assertions run at his actual size.
- **The naming call** above.
- **Everything already owed**: every tag from `v0.1.0`, the `Cargo.toml` version (still `0.1.0` after twelve arcs, so an installed binary lies about itself), the game fixture, the Ptyxis and i2c rows.

### Two things Matt asked for at the end, which changed how sessions work

**A summary that leads with the "so what".** His words on the closing message: *"it doesn't explain the so what aspects. it is too hard to follow and understand the why's and what's after a long season."* A faithful inventory of fixes is unreadable after hours of work he did not watch. `CLAUDE.md` now says to lead with what changed for him and why it matters.

**This journal, and a hook that will not let one be skipped.** The design constraint worth remembering: a hook is a shell command with no model behind it, so it can append a git log but never the narrative — and `SessionEnd` fires after the session is already over, too late to ask for anything. So the obligation lives in `CLAUDE.md` (which every session reads) and the enforcement is a `Stop` hook that exits 2, which sends its message back to the model as an instruction. `Stop` fires at the end of every *turn*, not the session, so two guards hold it to one interruption: the built-in loop flag and a per-session marker. It fired for real, once, on the backlog commit below — which is the intended behaviour, if slightly earlier in a session than is comfortable. Revisit the trigger if it proves mistimed.

**The backlog now says what comes next.** Every open item carries a `P1`–`P4` tag and the header opens with an ordered *Next up* list, because the file was grouped by *kind* — which is right for looking things up and useless for choosing. The tags are defined by *when*, not by vague importance: `P1` is the shortlist and nothing else, `P4` is blocked on something outside the repo. Pulling an item into an arc is still a DECISIONS entry; a tag is a forecast. **And fifteen items were rescued from under the "Won't do" heading**, where they had drifted by accident — the game fixture, the untested nvidia-smi fallback, `shot --config` and a dozen more are postponed, not refused, and a session reading that heading would have concluded we had decided against them. They have their own section now.

### Where to pick up

**Arc 13 is in flight as this entry is written**: the backlog's two P1 items, designed by a Fable session under D36 because both change a Rust contract (the model had switched to Opus mid-session, so the design was delegated rather than taken): **typed source options** (`from_table` returning `Result` — a wrongly *typed* value is still silent today) and **the dead `[store]` section** (`history = "1h"` silently gets ten minutes, and two further spec claims ride on the unimplemented `max_mb`). The `disk` component remains the one genuinely missing vertical.
