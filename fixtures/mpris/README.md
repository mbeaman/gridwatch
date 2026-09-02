# MPRIS metadata fixtures (§12, brief arc 6)

Recorded with `busctl --user get-property <bus> /org/mpris/MediaPlayer2
org.mpris.MediaPlayer2.Player Metadata` on torch (a safe read-only probe —
an agent never starts or controls a player), then written in the decoder's
own value shape so the tests need no D-Bus:

| key | shape | example |
|---|---|---|
| `str` | `s` or `o` | `{"str": "/org/mpris/MediaPlayer2/firefox"}` |
| `strs` | `as` | `{"strs": ["CthuLuck"]}` |
| `int` | `x`/`t`/`i`/`u` | `{"int": 7170000000}` |
| `float` | `d` | `{"float": 1.5e7}` |
| `bool` | `b` | `{"bool": true}` |
| `other` | anything else | `{"other": null}` |

| file | what it pins |
|---|---|
| `firefox-youtube.json` | Firefox on a YouTube tab, 2026-09-02: `xesam:artist` a one-element array, `xesam:album` an empty string, `mpris:trackid` an object path, `mpris:length` in µs, `mpris:artUrl` a `file://` PNG |
| `stream-no-length.json` | a live stream: no `mpris:length` at all ⇒ stream mode |
| `no-title.json` | a local file with no `xesam:title` ⇒ the title falls back to the URL's last part |
| `multi-artist.json` | two artists joined, and a `data:` art URL |
