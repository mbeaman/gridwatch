# Keys

**The complete list is [`docs/KEYBINDINGS.md`](../docs/KEYBINDINGS.md)**, which
is generated from the same table that draws the key bar at the bottom of the
screen, so the two cannot disagree. `?` shows it in the app, grouped by where
each key applies.

What is worth knowing before you read it:

**Focus, and who gets the keypress.** `hjkl` (or the arrow keys) moves focus
between tiles. Focus alone does not give a tile your keystrokes — press `Enter`
to hand them over, and `Esc` to take them back. This is why `q` is documented as
"quit **when no component holds the keys**": a tile that has them keeps `q` too,
so `Esc` first.

**`z` zooms**, and zooming is not just bigger. Several tiles have a zoom-only
`full` tier that is a different thing entirely — the CPU tile's is the whole
interactive htop, the GPU tile's adds sortable columns and a signal menu. If a
tile seems to be missing a feature you expect, zoom it.

**`e` is edit mode**, where you move and resize tiles with the keyboard, and `w`
saves to `layout.toml`.

---

*This page is a stub. It will grow a short "the ten keys worth memorising"
section; the generated reference covers everything in the meantime.*
