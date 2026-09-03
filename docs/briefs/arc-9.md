> **Implementation brief — arc 9 (v0.9.0 "ship it"). Written 2026-09-02 (D59) after arc 8, under Matt's autonomous mandate (D54): gate → scoped commits → push → CI watched; tags, releases and anything else outward-facing stay his and are recorded as owed, never faked.** Prereq: arc 8 on `main` (CI green, `ab2fa10`).

# Arc 9 — the last arc before 1.0: say what it does, prove what it costs, and make it installable

Two sessions, and **the split is not by size — it is by what this machine can prove.**

**9a is the half that can be gated here**: `gridwatch theme import`, one **global key table** replacing three hand-maintained copies of the key list, the four missing documents, `THIRD_PARTY.md` and `CONTRIBUTING.md`, and a criterion bench suite with committed baselines. Every row of it ends green in `scripts/gate.sh`.

**9b is packaging, and most of it cannot be exercised on torch**: there is no `nix`, no `nfpm`, no `makepkg`, and no `musl-gcc`. A `release.yml` nobody has run is the worst thing a "1.0 readiness" arc could ship, so 9b's first job is to make the release path runnable **without a tag**, and its honest output is a list of rows owed to Matt for the ones that still cannot be.

Doing 9a first means the arc has landed something real before it touches the half that ends in "owed".

**Read first:** `docs/PLAN.md`; `docs/ROADMAP.md` arc 9; `docs/ARCHITECTURE.md` §10 (UX and keys — the prose that the key table must replace as the source of truth), §14 (packaging and CI as specified), §15 (the adding-a-component checklist that `docs/ADDING-A-COMPONENT.md` extracts); `docs/WORKSPACE.md`'s tree (which names the files this arc creates); `docs/BACKLOG.md`'s arc-1b review items (two of them are 9a's work); D33 (spec before code, generated docs drift-checked), D41 (byte-deterministic shots), D54 (the mandate).

## What was measured before this brief was written

Four things, because they change what the brief can say:

1. **The musl target works out of the box for pure Rust.** `rustup target add x86_64-unknown-linux-musl` and `cargo build --release --no-default-features -p gridwatch` links with no external toolchain and produces a **static-pie** binary (5.2 MB). Rust's self-contained musl is enough.
2. **`ring` is the only blocker for a full-featured musl build.** It compiles C, so `cc-rs` looks for `x86_64-linux-musl-gcc` and fails. It arrives through `rustls ← ureq ←` astral-watch's Prometheus exporter and the album-art fetch. The fix is one apt line in CI (`musl-tools`); this box does not have it, so **the full musl tarball is built in CI and never here**.
3. **musl and gnu render byte-identical frames.** `shot --format cells` from both binaries `cmp`s clean, so D41's determinism promise holds across libc as well as across machines.
4. **A static musl binary will not be able to `dlopen` NVML.** This is not a bug to fix — it is a property of static linking, and the gpu source already has the path: `LibloadingError` → the `nvidia-smi` CSV tier. It must be **said out loud** in the release notes and asserted in CI (`doctor --offline` on the musl binary shows `✗ Nvml` with its fix line), because a musl user who is not told will file it as a bug.

## Seam decisions made in this brief (D59) — implement as written, escalate if they do not fit

1. **One global key table (`gridwatch-ui`, seam 13).** The list of global keys exists **three times** today and nothing keeps them equal: the key bar is one hard-coded string (`app.rs`), the `?` overlay is a hand-written array beside it, and `ARCHITECTURE.md` §10 is prose. Arc 9 makes one table the source of truth — `ui::keys::GLOBAL: &[Binding]`, where `Binding { keys, does, mode }` and `mode` is `Always | Grid | Edit | Showcase` — and derives all three from it: the **key bar** is built from the table and drops whole entries from the right rather than clipping mid-word (this closes an arc-1b review item that has been open since v0.1.0), the **`?` overlay** renders the table grouped by mode, and **`gridwatch keybindings` writes `docs/KEYBINDINGS.md`**, drift-checked by `scripts/shots.sh --check` exactly as `KEYS.md` and `COMPONENTS.md` are. A component's own keys already live in `Manifest.keys` and are already correct; the generated document joins the two, so a component that adds a key documents it by declaring it.
2. **`theme import` writes a file and never applies one.** `gridwatch theme import <FILE> [--name NAME] [-o PATH]` reads an **alacritty** TOML `[colors]` block, a **wezterm** TOML `[colors]` block, or a **base16/base24** YAML scheme, maps the sixteen ANSI colours plus foreground/background onto gridwatch's nineteen roles and eight gradients, and writes a theme that **validates against `theme.schema.json`** and passes the loader. It prints the WCAG contrast report and any warning the loader raises, so a scheme that cannot make readable muted text says so at import rather than at first use. Output goes to stdout unless `-o` is given; it **never** writes into `~/.config/gridwatch` on its own, because a command that silently edits a person's config is not a good citizen. An input it cannot recognise is an error naming the three formats, not a guess.
3. **Benches are criterion, committed with baselines, and are never a commit gate.** `cargo bench` does not join `scripts/gate.sh`: it would make the inner loop slow and a timing assertion on a shared machine is a flake generator. The suite covers the three things whose cost the design actually rests on — `Store::apply` over a realistic batch, `resample` over a full ring, and one whole-frame render of the Overview — and its numbers are recorded in `docs/PERFORMANCE.md`'s own table with the machine and date, so a regression is visible to a person rather than to a red build.
4. **`release.yml` must be exercisable without a tag.** A workflow that only runs on `v*` is untested code at the exact moment it matters. It gets a `workflow_dispatch` trigger and a **build-only job that runs on every push to `main`**: it builds the gnu tarball, builds the musl tarball with `musl-tools` installed, builds the deb and the rpm, and then **asserts them** — `tar -tzf` lists the binary and the licence, `dpkg-deb --contents` lists `/usr/bin/gridwatch`, the musl binary runs `--version` and `doctor --offline`. Only the tag path uploads anything, and **nothing in this arc ever publishes**: no `gh release create` outside the tag trigger, no crates.io, no AUR push. Tagging is Matt's.
5. **crates.io is a recorded decision, not code.** It is blocked twice over (crates.io forbids git dependencies; astral-watch's own publish was deprioritised — `BACKLOG.md`). The roadmap row says "decided", so the deliverable is a `DECISIONS.md` entry and a documented `cargo install --git` path in the README, not a publish.
6. **The Nix flake and the AUR PKGBUILD are written but marked unverified.** Neither tool exists here and neither is worth installing on Matt's workstation for one check. They ship with a comment saying so and a row in `PERFORMANCE.md`'s owed list — the same treatment every unverifiable row in this project has had.
7. **`docs/ADDING-A-COMPONENT.md` extracts §15 and is checked by following it.** Not a paraphrase: the document is written by walking the checklist against the newest real component (`sensors`, arc 5b) and naming the file and line each step lands in. A step that turns out to be wrong is a spec fix, not a doc fix.

## Session 9a — the half that can be proven here
1. The global key table (seam 1): `ui::keys`, the key bar rebuilt from it, the `?` overlay rebuilt from it, `gridwatch keybindings`, `docs/KEYBINDINGS.md` in `shots.sh --check`. A test asserts the key bar drops whole entries and never a partial word at 100, 118 and 250 columns.
2. `gridwatch theme import` (seam 2) with fixtures for all three formats under `fixtures/themes/import/`, a round-trip test (import → load → the loader accepts it → the swatch snapshot is stable), and the contrast report on stdout.
3. The four documents: `docs/ADDING-A-COMPONENT.md` (seam 7), `docs/LAYOUT.md`, `CONTRIBUTING.md`, `THIRD_PARTY.md` (generated from `cargo tree`/`cargo deny`'s licence list, so it cannot go stale silently).
4. The bench suite (seam 3) and its baselines in `PERFORMANCE.md`.
5. README: per-theme screenshots from `shots.sh` for every built-in, and the status paragraph rewritten for a reader who has never seen this repo.
6. Gate, review, CHANGELOG, push.

## Session 9b — packaging, and an honest list of what could not be proven
1. `release.yml` (seam 4) with its no-tag build job, and the musl row's `musl-tools` line.
2. `packaging/nfpm.yaml` (deb + rpm, `Recommends: pipewire-bin`), `packaging/aur/PKGBUILD`, `flake.nix` (seam 6).
3. The container acceptance — a fresh Ubuntu image with only `build-essential` and `pkg-config` builds every feature — run **once**, `nice`d, and only if no game is running (`MACHINE.md`); otherwise recorded as owed.
4. The crates.io decision (seam 5) and the `cargo install --git` path in the README.
5. Gate, review, CHANGELOG, push. **`v0.9.0` is not tagged.**

## Gotchas already verified — do not rediscover
- The key bar's string lives at `app.rs`'s `hints` binding and the `?` list is the `overlay::help(&[…])` call a hundred lines below it. Both go.
- `Manifest.keys` is already populated for every component that has keys (28 `KeyHint`s across the tree) and `component_info` already prints them — `KEYBINDINGS.md` joins that, it does not re-collect it.
- `scripts/shots.sh --check` is the existing drift gate and already regenerates `KEYS.md` and `COMPONENTS.md`; a third generated file is one more line in it and one more in CI's `docs` job.
- `packaging/udev/90-gridwatch-rapl.rules` already exists (D35 decision 10) — the packaging session installs it as an **optional** file, never by default.
- `cargo deny`'s allowed-licence list in `deny.toml` is the licence set `THIRD_PARTY.md` must agree with.
- musl: see "What was measured" above. Do not spend a session rediscovering that `ring` needs a C compiler.

## Escalate to Fable (do not improvise)
Any change to `Binding`/`Manifest.keys` beyond seam 1; any change that would make `cargo bench` a commit gate; anything that publishes (crates.io, AUR, a GitHub release) — that is Matt's, not a seam question; any change to `theme.schema.json` that an imported theme would need in order to validate (that is a theme-loader seam, D52).

## Done when
The gate is green; one table feeds the key bar, the help overlay and a generated `docs/KEYBINDINGS.md` that CI drift-checks; `theme import` turns all three foreign formats into a theme the loader accepts and reports its contrast; the four documents exist and `ADDING-A-COMPONENT.md` was written by following it; benches run with committed baselines and are not a gate; `release.yml` builds and *asserts* its artifacts on an ordinary push; the deb, rpm, PKGBUILD and flake exist with the unverified ones marked; the crates.io decision is recorded; and `v0.9.0` is **not** tagged, because tags are Matt's.


---

## What was owed at the end (2026-09-03)

Recorded here as well as in `PLAN.md`, because a brief that ends without saying what it did not do is the kind of document this project exists not to write. Three of the release triggers, only one is proven: `workflow_dispatch` was dispatched and green; `pull_request` on a packaging path is **written and unfired** (proving it means opening a PR, which is outward-facing); the tag path has never run because nothing is tagged. The PKGBUILD and the flake are written and marked UNVERIFIED in their own first lines.
