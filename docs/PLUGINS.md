# Writing a gridwatch plugin

A plugin is a program. gridwatch runs it, sends it JSON lines on its
standard input, and reads JSON lines from its standard output. That is the
whole interface — there is no library to link, no ABI to match, and no
language requirement. The example that ships with this repo is 140 lines of
Python with no dependencies:
[`plugins/examples/weather.py`](../plugins/examples/weather.py).

A plugin can be a **source** (it publishes metrics), a **component** (it
draws a tile), or both.

## The shortest possible plugin

```python
#!/usr/bin/env python3
import json, sys

def say(m):
    sys.stdout.write(json.dumps(m) + "\n")
    sys.stdout.flush()          # line by line, or the host waits forever

sys.stdin.readline()            # the host says hello first
say({"kind": "manifest", "manifest": {
    "kind": "hello", "name": "hello", "contract": 1,
    "tiers": [{"name": "badge", "min": {"w": 8, "h": 3}}],
}})
for line in sys.stdin:
    ask = json.loads(line)
    if ask.get("kind") == "render":
        say({"kind": "view", "instance": ask["instance"],
             "tree": {"text": [[{"role": "text", "text": "hello"}]]}})
```

Then, in `config.toml`:

```toml
[[plugins]]
id = "hello"
argv = ["python3", "/path/to/hello.py"]
```

and in `layout.toml`, place it like any other tile — the kind is
`<id>.<the kind your manifest declares>`:

```toml
place = [{ id = "greeting", kind = "hello.hello", at = [0, 0], size = [1, 1] }]
```

## The conversation

Every line is one JSON object with a `kind`. The full grammar is
[`schema/exec.schema.json`](../schema/exec.schema.json), which the host
validates against before it reads anything — if your line does not match,
it is refused with a reason, and three refusals stop your plugin.

| Direction | `kind` | When | What it carries |
|---|---|---|---|
| host → you | `hello` | once, first | the contract number, this machine's capabilities, the metric names already taken |
| you → host | `manifest` | once, in reply | what your tile is, its tiers, what it publishes ([`schema/manifest.schema.json`](../schema/manifest.schema.json)) |
| you → host | `sample` | any time | one reading: `key`, optional `label`, optional `at`, `value` |
| host → you | `render` | when your visible tile's inputs changed | `instance`, `tier`, `inner: {w,h}`, `now`, `focused`, `captured` |
| you → host | `view` | in reply to `render` | the tree to draw ([`schema/view.schema.json`](../schema/view.schema.json)) |
| host → you | `key` | while your tile is captured | one keystroke |
| you → host | `command` | any time | `toast`, `page` or `zoom` — a small, closed set |
| you → host | `status` | any time | `starting` / `ok` / `degraded` / `unavailable` / `stopped`, with a reason |
| you → host | `log` | any time | a line for the log file, never the screen |

Read your input in a loop and block on it. A plugin that polls costs the
machine something for nothing; a plugin that blocks on `stdin` costs
nothing at all while it waits.

## What your manifest must get right

The host refuses a manifest it could not place, and tells you why:

- **The first tier must fit an 8×3 tile.** That is the smallest tile this
  grid can make, and a tile that cannot draw at its smallest size would be
  blank at sizes a person can reach.
- **Tiers are cumulative and poorest first.** Each one may need more room
  than the last, never less. Tier 0 is what you draw in a corner; the last
  one is what you draw when someone zooms.
- **The first tier cannot be `zoom_only`.**
- **Your keys are `<source>.<metric>`, lower case**, with `_` for spaces.
  The host prefixes every key you publish with your plugin's `id`, so you
  cannot collide with a built-in metric or with another plugin — publish
  `weather.temp_c` under id `outside` and the store holds
  `outside.temp_c`.

## What you draw

A `view` is a **tree**, not pixels. You name a role (`text`, `text_muted`,
`accent_primary`, `ok`, `warn`, `crit`, …) and the host's current theme
decides what that looks like. You cannot choose a colour, a glyph set, or a
cell position, and that is deliberate: it is why a plugin cannot break the
dashboard's look, and why your tile follows along when someone switches
themes.

The shapes available to you are the ones the built-in tiles use — text,
key/value pairs, gauges, bars, sparklines, charts, tables, big digits, and
stacks of those. [`schema/view.schema.json`](../schema/view.schema.json) is
the list.

## What the host does about a plugin that misbehaves

None of this is about trusting you; it is about the dashboard staying up.

- **No shell.** Your `argv` is a program and its arguments, passed
  verbatim. Nothing in a config file is ever interpreted by a shell.
- **A bounded reader.** A line over 1 MiB is refused without being read
  into memory.
- **Three strikes.** Three malformed messages and your plugin is stopped
  rather than restarted — a plugin that cannot speak the protocol will not
  start speaking it on the fourth attempt, and a restart loop is how one
  broken plugin becomes a busy machine.
- **Resource limits.** The child gets `RLIMIT_AS` and `RLIMIT_CPU`
  (defaults: 256 MiB, 600 s), configurable per plugin.
- **A clean environment.** Your process gets `PATH` and the contract
  number, and nothing else of the host's.
- **Restart backoff.** 1, 2, 4, 8, then 30 seconds.

A `status` of `unavailable` with a reason is **not** a strike. It is the
right way to say "I cannot work on this machine", and the host will show
your reason and your hint on the tile instead of the tile's contents.

## Debugging

- `gridwatch config check` lists the plugins it would start and the
  manifests it would accept.
- Your `stderr` goes to the log (`$XDG_STATE_HOME/gridwatch/gridwatch.log`),
  never to the screen — the screen is an alternate buffer and a stray
  `print` would corrupt it. Use `log` messages, or stderr, not stdout.
- `fixtures/plugins/handshake.jsonl` is a complete, valid conversation you
  can diff against; `fixtures/plugins/bad.jsonl` is a collection of lines
  the host refuses, each with the reason it gives.
