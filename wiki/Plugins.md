# Plugins

A tile can be a program in **any language** that can write JSON lines to a pipe.
No recompile, no Rust.

**[`docs/PLUGINS.md`](../docs/PLUGINS.md) is the guide**, and it explains the
whole protocol against a working example in 140 lines of Python.

The one design point worth repeating here: a plugin returns a **view tree** — "a
table with these columns, a gauge at this value" — and never cells or colours.
The theme's renderer draws it. That is why a plugin cannot break your theme,
cannot write outside its tile, and looks native in all seven schemes without
knowing any of them exist.

What the host does on your behalf: no shell, a cleared environment, memory and
CPU limits, a line-length cap, schema validation before anything is read,
unknown message kinds refused, three strikes before it gives up, a bounded queue
that drops oldest, and a runaway check that kills a plugin spending half a core
for ten seconds.

---

*This page is a stub. It will grow a worked example and the packaging story;
[`docs/PLUGINS.md`](../docs/PLUGINS.md) is complete in the meantime.*
