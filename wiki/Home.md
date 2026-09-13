# gridwatch

A dashboard for a terminal. The things you would normally watch in four or five
separate windows — processor and memory, the graphics card, the drives, the
network, temperatures, what is playing — on one screen you can rearrange, in a
colour scheme you pick.

[![the Overview in retrowave](../docs/img/overview-retrowave.svg)](../docs/img/overview-retrowave.svg)

It reproduces what [htop](https://htop.dev), [nvtop](https://github.com/Syllo/nvtop)
and [astral-watch](https://github.com/mbeaman/astral-watch) show, down to their
formulas, and adds tiles those tools do not have. It runs over SSH, costs
almost nothing to leave open, and every tile renders from a chip the size of a
postage stamp up to the whole screen.

## Start here

| | |
|---|---|
| **[Installing](Installing.md)** | What you need, how to install it, and the first two commands to run |
| **[The tiles](Tiles.md)** | All eleven, with a picture of each and what its numbers mean |
| **[Configuring](Configuring.md)** | The two config files, adding a tile, and the one tile you have to add yourself |
| **[Troubleshooting](Troubleshooting.md)** | A tile is blank, a number is a dash, a permission is missing |

And the shorter pages: **[Themes](Themes.md)** · **[Keys](Keys.md)** ·
**[Plugins](Plugins.md)**.

## The one thing worth knowing up front

**Nothing here guesses, and nothing pretends.** If a tile cannot reach its
hardware it prints the reason and the command that would fix it, instead of
drawing a plausible zero. If a reading is stale it says `STALE` rather than
holding a flat line that looks steady. If a number cannot be trusted — the disk
tile's utilisation is the standing example — it is drawn under a name that says
so, with the honest companion figure beside it.

Run `gridwatch doctor` and it prints that reasoning for every capability at once.

## If you are here to change it rather than use it

This guide is for using gridwatch. The specification is in
[`docs/`](../docs/): [`ARCHITECTURE.md`](../docs/ARCHITECTURE.md) for the design
and its contracts, [`DECISIONS.md`](../docs/DECISIONS.md) for why each call was
made, [`PERFORMANCE.md`](../docs/PERFORMANCE.md) for the measured ceilings that
gate every commit, and
[`ADDING-A-COMPONENT.md`](../docs/ADDING-A-COMPONENT.md) for writing a new tile.
