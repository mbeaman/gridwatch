# Journal fixtures (§4.5, §12.4)

Recorded on torch with the release binary under a real pty, tables off, the
default embedded config (`XDG_CONFIG_HOME` pointed at an empty directory), so
every line is what `gridwatch run --record` writes and nothing else:

```sh
(sleep 63; printf q) | script -qfec "stty rows 70 cols 250; exec target/release/gridwatch run --record fixtures/journals/torch-idle.jsonl" /dev/null
```

| file | recorded | what the box was doing | lines | size |
|---|---|---|---|---|
| `torch-idle.jsonl` | 2026-09-01, arc 2b (re-recorded with the gpu source; 2a's cpu-only file replaced) | idle desktop, Ptyxis, no game; the Overview's cpu tile focused (500 ms cadence), the gpu tile visible (500 ms fast tier, 1 s slow tier) | see `wc -l` | — |
| `synth-overload.jsonl` | 2026-09-02, arc 3a | `gridwatch run --demo 42 --record …` for 63 s: every synth, the pins synth's scripted overload (pins 1–2 at 9.5 A from 20 s to 40 s) with its `al` lines — `pins/overload` raised at 21.5 s, resolved at 50 s — so the replay test finds the banner on page 2 at the scripted instant | see `wc -l` | — |
| `torch-audio.jsonl` | 2026-09-02, arc 5a | the live `audio` source for 60 s on an idle desktop with **nothing playing** (Firefox open and silent; the graph's quantum 0 in `pw-top`) — the **silence path**: the sink Record (the default USB headphones sink, serial 61, `suspended`), `audio.level` silent from the first tick, zero bands at the 2 Hz silence cadence (123 band samples), `gw-audio`/`gw-audio-io`/`pw-record` at 0.0 % CPU. Recorded with `[sources.pins] source = "exporter"` at a dead port so the pins source never opened `/dev/i2c-*` (MACHINE.md). The "something playing" recording is Matt's — an agent does not start players | see `wc -l` | — |
| `torch-game.jsonl` | **owed** | a game running — Matt starts it; an agent must not (MACHINE.md). No game was up during 2a or 2b | — | — |

Record the game fixture with the same command, `torch-game.jsonl` in place of `torch-idle.jsonl`, while the game runs.
Validated line by line against `schema/journal.schema.json` by `scripts/check-schemas.py`.
