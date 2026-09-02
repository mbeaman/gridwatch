# Journal fixtures (§4.5, §12.4)

Recorded on torch with the release binary under a real pty, tables off, the
default embedded config (`XDG_CONFIG_HOME` pointed at an empty directory), so
every line is what `gridwatch run --record` writes and nothing else:

```sh
(sleep 63; printf q) | script -qfec "stty rows 70 cols 250; exec target/release/gridwatch run --record fixtures/journals/torch-idle.jsonl" /dev/null
```

| file | recorded | what the box was doing | lines | size |
|---|---|---|---|---|
| `torch-idle.jsonl` | 2026-09-01, arc 2a | idle desktop, Ptyxis, no game; the Overview's cpu tile focused (500 ms cadence) | 129 | 710 KB |
| `torch-game.jsonl` | **owed** | a game running — Matt starts it; an agent must not (MACHINE.md) | — | — |

Session 2b re-records both with the gpu source and replaces them (brief 2a task 6).
Validated line by line against `schema/journal.schema.json` by `scripts/check-schemas.py`.
