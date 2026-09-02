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
| `torch-game.jsonl` | **owed** | a game running — Matt starts it; an agent must not (MACHINE.md). No game was up during 2a or 2b | — | — |

Record the game fixture with the same command, `torch-game.jsonl` in place of `torch-idle.jsonl`, while the game runs.
Validated line by line against `schema/journal.schema.json` by `scripts/check-schemas.py`.
