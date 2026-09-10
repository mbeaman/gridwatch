# Session journal

> One entry per working session, newest first. This is the only document that records **what a session was like** — what was tried, what the reviews caught, what turned out to be wrong, and what is owed to Matt when it ends. The others answer different questions: `DECISIONS.md` says *why a choice was made*, `CHANGELOG.md` says *what shipped*, `ROADMAP.md` says *what an arc promised*, `PLAN.md` says *where the project stands right now*. A journal entry is allowed to record a dead end, a wrong assumption or a near miss, because those are the things no other file has a place for.
>
> **Write the entry before the session ends**, while the work is still in context, and commit it with the session's last commit. Lead with what changed for Matt and why it matters — not an inventory of edits (Matt, 2026-09-07). Keep it to what a reader six months from now would need.

---

## 2026-09-09 — arc 15: what a tile draws with the room

**Models:** Opus 5 for the whole arc (D65, the ARCHITECTURE edits and the brief were written by an Opus session the same day, because Fable was rate-limited). **Shipped:** arc 15, 15a in five commits and 15b in six. **Nothing tagged. No review yet** — that is the next session's, along with arc 14's, and both ROADMAP boxes are still open.

### What changed for Matt

**The tile that was worst was not the one anybody was looking at.** The whole arc came out of "it appears broken if the console resolution is too wide", and the sharpest defect in it was on the *reference* 250×70 screen and had been since arc 5b: the SENSORS tile put a reading's number **eighty-seven cells** from the sensor it belonged to. Its one elastic column sat *before* the fixed ones, so all the spare width went between a row's identity and its value. That table now ends where its content ends, and there are warn/crit bars in the room it gives back — the bars §8 has promised since arc 5b. The tile went from 0.204 to 0.716 of its cells at 250×70.

**One function in the renderer fixed six tables.** An elastic column grows to the widest cell it holds — over *every* row, never the visible page, because a column that changes width as you scroll is worse than one that stretches — and stops. `sensors`' `sensor`, `net`'s `iface`/`local`/`remote`, `audio`'s sink picker and `winamp`'s playlist were all fixed without touching a component, and the four tables that already obeyed the rule are byte-identical below their content width.

**Charts have an axis at last.** `PARITY.md` row 68 has said "fixed 0–100 % axis with 25/50/75/100 ticks — in" since arc 2b. The row claimed the axis and shipped only its *range*: nothing drew a tick. The renderer draws them now, from `Bounds.y`, under the series so ink always wins the cell. And `Series.label`, which has travelled in every `View::Chart` since arc 2b and was **read by nothing**, is printed at each series' newest point — a name on its line instead of a legend sixteen rows above the ink.

**The network tile draws something for the first time.** Its `table` tier was an interface table, a probe strip and a footer, in a band a workstation's three interfaces could never fill. The band is content-sized now and the rows it gives back are the mirrored rx/tx chart the arc-7 spec named and D62 deleted rather than built. The renderer's midpoint gridline *is* its zero line, which is the whole reason 15a had to land before 15b. Its connection table also had a real bug: it computed its scroll viewport from the tile's inner height while being drawn in a band roughly half that, so a cursor past the fold scrolled to a page that did not contain it.

**And the GPU's spec pane stopped eating rows it could not use.** A 24-wide column across the whole band was thirty blank rows at 480×135 and, worse, silently cut `driver` and `vbios` at the reference size, where the band is shorter than the list. It is a wrapped strip under the chart now: every row prints, and the chart has the full width.

### The two numbers that went the wrong way, and why that is the point

D65 §9 says the cell fraction is a collapse guard and never a proof. This arc proved it twice.

**The sensors tile's *column* coverage at 480×135 is 1.000 before the fix and 1.000 after.** Every column carried ink either way — they were simply eighty-seven cells apart, with a temperature at column 104 and the sensor it belongs to at column 17. The instrument that was supposed to find wide-terminal emptiness reported a perfect score on the worst instance of it. *(This entry first made the same point with the 250×70 numbers and got them wrong: there the column coverage did move, 0.285 → 0.870. The arc's review caught it against the arc's own recorded table, and the corrected claim is the stronger one — a metric reading 1.000 while the tile is broken is worse than one reading 0.285.)*

**The gpu tile's row coverage fell**, 0.554 → 0.415, in the same change that raised its cells 0.131 → 0.183: sixteen rows of spec down the right-hand edge became two rows under a chart that gained 24 columns. Its row floor comes down with a sentence saying why. A floor that is re-fitted upward every arc measures nothing; one that moves *down* with a written reason is at least honest.

### Two things the decision got wrong, and one it got wrong in our favour

**D65 §5's table rule does not survive contact with the tree it was written against.** "Every `View::Table` leaf's summed drawn width must not exceed its natural width plus one pad per column" fails on `disk` at four times its tier minimum: nine columns declaring fifteen cells more than the demo fixture fills. A fixed width is a *declaration* sized for the widest value a column can ever hold — `net`'s `state` is `Fixed(10)` for `no carrier` against a fixture whose interfaces are all `up`, `winamp`'s `artist` 18 for an eight-character name — and no per-column slack that passes those also catches a `Fixed(999)`; it would just be calibrated on the synths. So the assertion is two rules: an **elastic** column is checked exactly against its content (no allowance, a stretch is a failure) and a **fixed** one is checked for the damage a `Fixed(999)` actually does — pushing the columns after it off the right edge, where the renderer skips them, which is the same defect D57 amendment 19 fixed for elastic columns.

**D65 §10's per-leaf doubling test is not satisfiable for every drawing kind, because freezing the view freezes the component's data.** Rendering a leaf alone at double its rect does not re-run `view`, so a `Sparkline` with one sample per column draws the same samples right-anchored and scores 1.0×; `Bars` draws `values.len()` bars whatever the width, and `audio`'s `mini` fixes that at ten by design; `Segmented` draws its unfilled part as blank space, so htop's nearly-empty `SWP` measures 16 cells at 30 columns and 16 at 60; and the audio scope's braille chart measures **1.4978×** across a doubled width, because doubling the columns halves the vertical excursion per column. Each of those is a property of the *renderer*, not of a caller, so the oracle asks two questions instead of one — does the ink **span** the rect it was given (which is D62's actual defect, and which every drawing must answer), and does doubling an axis it can genuinely use double its cells — with every exemption named once in `testkit::drawing_oracle` rather than at seven call sites. That is a deviation from the decision and it is deliberate: the alternative was one assertion weakened until it meant nothing, which is the road `assert_grows_with_area` walked to five exclusions in a single arc.

**Trap 5 was wrong in our favour.** The brief predicted the mirrored chart would fail the growth sweep's height axis as `gpu` `charts` does. It does not — 1.54× — because that band absorbs every row the tier's constant text leaves. Both axes are asserted. Both are close to the bar (width 1.50×) and the comment says so, because the tier is mostly content-sized text plus one chart and only the chart grows.

### The same bug twice more, in the two tiers nobody looks at

The pre-report review found that **both** `full` tiers still guessed their
viewports after the arc had just finished fixing exactly that on the grid
tiers. `net`'s zoom-only connection browser — the one tier where the `Fill`
band is most of the body — was told it had `inner.height / 3` rows: at 248×66
that is 22 against a 53-row band, so at the end of a sixty-row list it drew
**22 rows and thirty blank ones**, measured. And `sensors`' `full` passed
`temps.len()` as its body, which makes the scroll arithmetic collapse to a
constant zero: the cursor simply walks off the bottom of any rect shorter than
the reading list, which torch's eight readings hide and forty do not. Both are
fixed and both have a test that was watched to fail against the old code before
it was trusted.

The lesson is narrow and worth keeping: **when a rule is applied to "every X",
grep for X rather than for the ones the brief listed.** D65 §5 says "both
scroll viewports", the brief named the `conns` tier's two, and there were four.

### The measurement that would have been a lie

The first P19 "before" run, taken on this tree straight after a release build, read 753.8 µs / 1.4596 ms — 43 % over the arc-12 numbers, for reasons that have nothing to do with any code. The after-run read 549 µs / 1.088 ms, and comparing the two would have let this arc claim a **26 % speed-up** from a change that only adds drawing. The honest number came from checking out `75aa940` in a scratch worktree and benching it minutes before: **521.9 → 549.5 µs and 1.0400 → 1.0876 ms, +5 %.** Arc 14 lost two hours to a stale release binary and wrote down "any measurement of a component must first prove the component drew"; the sibling rule is that a before-number taken at a different time is not a before-number.

### What is owed to Matt after this session

- **The arc-end adversarial review**, with D65 §9's lens: render every component tier at 480×135 and 250×70 and answer the three questions in order. Arc 14's review is still owed too.
- **A pty case driving the connection cursor past the fold.** `demo::NetSynth` publishes five connections and no tier that shows connections can have a band shorter than five rows, so the fold is unreachable in a real terminal with the shipped fixture. The unit test drives it with sixty. Fixing the synths is its own backlog item because it churns every net snapshot.
- **`MATT_TERMINAL`** in `crates/cli/tests/smoke.rs` is still `None`: nobody has measured his real Ptyxis size, and a guess would pin nothing.
- **`v0.15.0`**, and every tag from `v0.1.0`.

---

### The review, and the pattern in what it found

Two lenses. The code held on nearly every point; what did not hold was an **instrument**, a **number**, and a **habit**.

**The assertion this arc offered as proof of its own rule could not fail.** `assert_tables_end_at_their_content` compared `table_widths(cols, rows, w)` against `table_natural_widths(cols, rows)` — and `table_widths` computes its cap by calling that same function with those same arguments. Both sides moved together. It could only break if someone deleted the `min`, and the two failures the decision names most (measuring the visible page instead of every row, a byte metric instead of display columns) would have sailed through it. It measures from a rendered buffer now, against the rule rather than the arithmetic: *draw the same table twice as wide and its ink must not move right*. Removing the cap makes it fail; the version it replaced did not.

**A number this arc reported about itself was wrong, and the true one is stronger.** The record said the sensors tile's column coverage at 250×70 was "0.285 before and 0.285 after" — evidence that the metric was blind. It is 0.285 before and **0.870** after; the arc's own table said so two paragraphs above the prose contradicting it, and the reviewer caught the file arguing with itself. The point survives at the other size and lands harder: at **480×135 it is 1.000 before and 1.000 after**. A metric scoring a tile *perfectly* while a temperature sits ninety cells from its sensor is a better argument for three ordered questions than one scoring it 0.285.

**And the habit: a rule was applied to the instances the brief listed rather than to every instance.** Four times.

- The arc fixed four scroll viewports and wrote down the lesson "grep for X rather than trusting the brief's list". There was a **fifth**, in the file it had just rewritten.
- It made the header bar stop at the table's content and left the **selected row's** bar running to the rect edge.
- It gave the sensors table a bar column at the `table` tier; `full` sits above `table` and drew neither the bars nor the chart, so `z` on a wide terminal made the tile **strictly poorer than the one it zoomed** — twenty rows of a hundred and thirty-one. No snapshot could see it, because at every size in the matrix that tile picks `chart` and the file called `sensors_zoom` never rendered `full`.
- It made net's interface band content-sized and left the **sensors** table band on `Fill` — sixty-one blank rows at 480×135, in the other tile it had just rebuilt.

And one where the grep itself was aimed one way: §1 says the free-text column is elastic and last, §2 grepped every table for *elastic before fixed*, and net's `process` column is *fixed where elastic belongs* — cutting `firefox-bin (50558)` mid-pid at every width from 250 to 800 columns with up to 550 empty beside it.

**What that suggests for the next arc.** A rule that ships with a list of the places it applies will be applied to the list. The instrument has to enumerate, not the brief — which is what `assert_tables_end_at_their_content` and `assert_every_drawing_grows` do now over the whole registry, and what nothing does for tiers, bands or viewports.

### Owed after this arc

The net chart's **zero line is the one gridline its ink erases** — 49 of 248 cells at 250×70, 7 of 248 in the demo, and at a 4–7-row band the user sees no line at all. D65's acceptance said it must read as a zero line and it does not; the fix is a rendering-order or role question, so it is backlogged rather than rushed. Also backlogged: `net` has no snapshot at all and no chart has a *cell* snapshot, so this arc's global renderer change is pinned in cells for no chart; the per-drawing oracle asserts *reach* for every drawing but doubling for two of ten variant/axis pairs, which is honest but narrower than the record admitted; and a chart series is silently unlabelled when its label would collide.


## 2026-09-09 — arc 14: what the drives are doing

**Models:** Opus 5 for the whole arc (D64, the ARCHITECTURE edits and the brief were written by an Opus session the day before, because Fable was rate-limited). **Shipped:** arc 14, 14a in four commits and 14b in three. **Nothing tagged. No review yet** — that is the next session's, and the ROADMAP box for it is still open.

### What changed for Matt

gridwatch can finally answer "which drive is busy, how hard, and how much is that hurting". Nothing in the project had ever opened `/proc/diskstats`. There is now a `disk` source and a five-tier tile — read/write rates, IOPS, service times, the drive's temperature, and one column that deliberately disagrees with every other tool on the machine.

**The tile says `BUSY`, not `%util`, and that is the whole point of the arc.** Every disk tool prints field 13 of diskstats as utilisation. On an NVMe drive it means almost nothing: it is the share of wall time the queue was non-empty, so *one* outstanding I/O reads 100 % while a thousand slots sit free. Torch proved it over a live interval: one 537 MB direct read measured **524 MB/s at 7.3 % util** with `aqu-sz 0.38`. **The arc first proved it a second way and that way was wrong** — a derivation from lifetime counters gave "a mean of 1.94 I/Os, about 169 in flight whenever busy", and diskstats prints the service-time counters truncated to 32 bits, so `nvme0n1`'s had already wrapped and the figure was an artifact. The review caught it from the file's own arithmetic (the four service-time accumulators sum past 2³², and field 14 is that sum modulo 2³²). The argument survived; the evidence had to be replaced. **Prefer an interval to a lifetime counter, always.** So the column is `BUSY`, `Q` (the mean in flight) sits beside it, and the sentence explaining the difference is on the tile rather than buried in a decision file.

**A drive is a sysfs fact.** `/sys/block/<dev>/device` exists if and only if the device is real hardware — exactly 3 of torch's 55 `/sys/block` entries. htop's rule is a name-prefix skip that does not exclude `loop*`, which on this machine sums 52 squashfs devices into "the disks". `extra` opts the refused ones back in, and 16 devices is a hard cap, because nine scalars over all 64 diskstats lines would be about 22 MB of store against a 60 MB budget.

**And the drive's temperature comes from the sensors source, joined by device.** `disk.info{nvme0n1}.device` is `nvme0`; so is the hwmon chip's `ChipInfo.device`. Never by index — on torch hwmon0/1/2 are nvme1/nvme2/nvme0, and any agreement between the two numberings is a coincidence.

### The brief was wrong about the field guard, in a way that mattered

The brief said "guard `>= 20` fields while tolerating both more and fewer" and asked for "a 17-field pre-5.5 diskstats" fixture. Neither is right. The layout has grown twice: 11 stats before 4.18 (14 fields), 15 with the discard group (18), 17 with the flush group (20). **17 fields is a shape no kernel has ever emitted**, and a `>= 20` guard would have silently dropped `disk.discard_bps` on a real 4.18–5.4 machine that has it. The guard is per group now — the core needs 14 fields, `discard_bps` needs the whole discard group, a longer tail is ignored — and the fixture is a genuine 14-field pre-4.18 file.

The traps the brief *did* record all bit, and were cheap because they were written down: the 1-based field numbering (`io_ticks` is index 12 after splitting, not 9), the 512-byte sector rule, `canonicalize` erroring on a missing `device` link being the signal rather than a failure. One trap of the same class was **not** in the brief and is now in the code: **sysfs `size` is in 512-byte sectors too**, so `nvme0n1` reads 7 814 037 168 and a forgotten multiply prints 7.8 GB for a 4 TB drive — plausible, and wrong.

### Two hours lost to a stale binary, and what it cost

The performance run said the disk source was sitting at 0.5 wake-ups/s with the tile visible — a hidden cadence. Chasing that produced a good deal of confusion about layout resolution and demand levels before the real cause turned up: **`scripts/gate.sh --quick` does not build release**, and `target/release/gridwatch` still predated 14b. The tile in the measured run was a placeholder chip reading "arrives in a later arc". The lesson is small and worth keeping: **any measurement of a component must first prove the component drew**, and the cheapest proof is grepping the typescript for something only that tile says.

### P5 is over its ceiling, and it is not this arc's fault

Measured with the disk tile visible: **69.6 wake-ups/s against a ceiling of 40**. Without it: 71.7. `gw-disk` is 1.0 /s visible and 0.5 /s hidden — the tile costs half a wake-up a second. The row is over for two reasons, both older than arc 14 and both worth a decision:

1. **`gw-audio` at 37–40 /s.** P5's derivation budgets "audio idle 2", but the shipped Overview places the `viz` tile, so the DSP runs at 30 fps whenever page 1 is on screen. The derivation and the shipped layout disagree.
2. **`gw-sensors` at 14 /s on a 1 s cadence.** Σ Δ`voluntary_ctxt_switches` — the metric P5's own protocol prescribes — counts *every* yield, including a blocking sysfs read. The sensors source reads ~40 hwmon files a second and books ~14 switches per pass. The number is real; what it measures is not only what "wake-ups per second" suggests.

Neither is fixed here. Recording the disk source's contribution as an isolated number is the useful thing: the next session starts from evidence rather than suspicion.

A second measurement disagreement is recorded and not chased: **P6's HUD cross-check is off by 2×** (Δ`wchar` 10.6 kB/s against the stats log's own `bytes` at 4.9), where P6 requires 5 %. The recorded typescript sides with `wchar`, the row is inside its ceiling by either number, and nothing in this arc touches the byte accounting.

### The pty test that taught me something about diff streams

C.36 presses `1`–`4` to change the charted series and asserts the legend changed. It kept failing on `queue` while passing on `write` and `busy`. The typescript is a **diff stream**: `busy` → `queue` shares its second character, so the terminal is sent `q`, a cursor move, then `eue over …`, and the stripped text reads `qeue over`. The assertions are on the transitions whose *word length* changes, because then the rest of the line shifts and is redrawn whole. The four keys are pinned exactly in a unit test; the pty row only proves they arrive.

### Owed to Matt after this session

- **The arc-end adversarial review**, with the lens D64 asks for: grep the component for `std::fs` and for `Detail::`.
- **The write-side `iostat` cross-check under real load**, and the loaded tile-versus-`iostat` comparison — the read side ran once, bounded, and an agent does not write half a gigabyte to his boot drive.
- **Where `disk` belongs in the shipped `layout.toml`.** The Overview is full; every candidate slot displaces a tile a test or the performance protocol depends on. Acceptance used a sandbox layout.
- **P5 with the pins source on `auto`** (it opens `/dev/i2c-*`, so the run above pinned it to `exporter`), and every row beside the game.
- **`v0.14.0`**, and every tag from `v0.1.0`.

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
