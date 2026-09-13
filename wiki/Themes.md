# Themes

Seven built in. Press `t` to cycle them, or set `theme` in `config.toml`.

[`modern`](../docs/img/overview-modern.svg) ·
[`retrowave`](../docs/img/overview-retrowave.svg) ·
[`mono`](../docs/img/overview-mono.svg) ·
[`terminal`](../docs/img/overview-terminal.svg) ·
[`phosphor-green`](../docs/img/overview-phosphor-green.svg) ·
[`phosphor-amber`](../docs/img/overview-phosphor-amber.svg) ·
[`matrix`](../docs/img/overview-matrix.svg)

Two are deliberately colourless and are not a compromise: **`mono`** is the real
`NO_COLOR` theme, where severity is carried by weight and glyph rather than hue,
and **`terminal`** uses your terminal's own sixteen colours by name, so it
inherits whatever scheme you already have.

**`matrix`** is a showcase theme where only the rain draws: the rendered frame is
a mould the rain falls through, so a widget is the rain's memory of having fallen
through its shape. It has its own, looser performance ceilings, applied only
while the terminal is focused, and it freezes the moment you tab away.

Bring your own scheme:

```sh
gridwatch theme import ~/.config/alacritty/theme.toml -o mine.toml
```

It reads alacritty, wezterm and base16/base24 schemes, derives the roles a
foreign scheme cannot carry — muted and ghost text, the panel lift — then loads
what it wrote and prints a WCAG contrast report, so a scheme that cannot make
readable muted text tells you at import rather than in use.

---

*This page is a stub. It will grow the role vocabulary, glyph tiers and how to
write a theme from scratch; [`docs/THEMES.md`](../docs/THEMES.md) has the
reference in the meantime.*
