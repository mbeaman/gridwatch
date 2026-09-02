//! The Winamp tiers as view trees (§8): `status` (a play glyph, the marquee
//! and a two-cell posbar), `shade` (+ the clock and a mini spectrum),
//! `main` (+ big digits, the full spectrum, volume and the transport row),
//! `main+art` (+ the cover through the ui crate's halfblock painter) and the
//! zoom-only `full` (+ the playlist and the player list). Every colour is a
//! theme role; the art painter belongs to the ui crate.

use std::borrow::Cow;

use gridwatch_store::SourceState;
use gridwatch_store::keys::media::{Art, NowPlaying, PlayStatus};
use gridwatch_store::keys::{audio, media};
use gridwatch_ui::component::RenderCx;
use gridwatch_ui::halfblock::ArtPainter;
use gridwatch_ui::theme::{GradientId, Role};
use gridwatch_ui::view::{ColWidth, Column, Constraint, Dir, Line, Span, View};

use super::marquee::{clock, remaining, window};
use super::{TIER_ART, TIER_MAIN, TIER_SHADE, TIER_STATUS, Vis, Winamp};

/// The transport glyphs, in the unicode tier every theme has.
const PLAY: &str = "▶";
const PAUSE: &str = "‖";
const STOP: &str = "■";

pub fn render(w: &Winamp, cx: &RenderCx<'_>) -> View {
    let now = cx.store.record(&media::NOW).map(|(_, n)| n.clone());
    let Some(now) = now.filter(|n| !n.title.is_empty() || n.status != PlayStatus::Stopped) else {
        return idle(cx);
    };
    match cx.tier {
        TIER_STATUS => status_tier(w, cx, &now),
        TIER_SHADE => shade(w, cx, &now),
        TIER_MAIN => main_tier(w, cx, &now, false),
        TIER_ART => main_tier(w, cx, &now, true),
        _ => full(w, cx, &now),
    }
}

/// No player (or none playing): the idle skin, with the source's reason
/// when it has one.
fn idle(cx: &RenderCx<'_>) -> View {
    let st = cx.store.status(media::SOURCE);
    let line: Line = match st.state {
        SourceState::Unavailable | SourceState::Degraded => {
            let mut text = st.reason.as_deref().unwrap_or("unavailable").to_string();
            if let Some(h) = st.hint.as_deref() {
                text.push_str(" — ");
                text.push_str(h);
            }
            vec![Span::new(Role::Warn, text)]
        }
        _ => vec![
            Span::new(Role::TextMuted, format!("{STOP} ")),
            Span::new(Role::TextMuted, "no player"),
        ],
    };
    View::Stack {
        dir: Dir::V,
        children: vec![
            (Constraint::Len(1), View::Text(vec![line])),
            (Constraint::Fill(1), View::Empty),
        ],
    }
}

fn glyph(status: PlayStatus) -> &'static str {
    match status {
        PlayStatus::Playing => PLAY,
        PlayStatus::Paused => PAUSE,
        PlayStatus::Stopped => STOP,
    }
}

fn role_for(status: PlayStatus) -> Role {
    match status {
        PlayStatus::Playing => Role::Ok,
        PlayStatus::Paused => Role::Warn,
        PlayStatus::Stopped => Role::TextMuted,
    }
}

/// `Artist — Title`, the way a skin shows it.
fn full_title(now: &NowPlaying) -> String {
    if now.artist.is_empty() {
        now.title.clone()
    } else {
        format!("{} — {}", now.artist, now.title)
    }
}

/// The posbar: a gauge of the fraction, or the stream pattern when there is
/// no length to divide by.
fn posbar(now: &NowPlaying, cx: &RenderCx<'_>, text: Option<String>) -> View {
    match now.fraction(cx.now) {
        Some(f) => View::Gauge {
            label: Cow::Borrowed(""),
            value: f as f32,
            gradient: GradientId::Title,
            text: text.map(Cow::Owned),
        },
        None => {
            // Stream mode: no bar, the elapsed clock and a note instead.
            let mut line: Line = vec![Span::new(Role::AccentSecondary, "stream")];
            if let Some(t) = text {
                line.push(Span::new(Role::TextMuted, format!("  {t}")));
            }
            View::Text(vec![line])
        }
    }
}

/// The spectrum, from the audio source's bands; a flat skin without it.
/// `cols` is the width to fill: the classic nineteen bands are widened to
/// cover it, with a gap column once each band is three cells or more.
fn vis(w: &Winamp, cx: &RenderCx<'_>, cols: usize) -> View {
    if w.options().vis == Vis::Off || cols == 0 {
        return View::Empty;
    }
    let bars = BANDS.min(cols.max(1));
    let mut values = vec![0f32; bars];
    let mut have = false;
    for ch in 0..2u16 {
        if let Some((_, v)) = cx.store.vector(&audio::BANDS_KEY.idx(ch)) {
            have = true;
            for (i, out) in values.iter_mut().enumerate() {
                let a = i * v.len() / bars;
                let b = (((i + 1) * v.len()) / bars).max(a + 1).min(v.len());
                let m = v[a..b].iter().cloned().fold(0f32, f32::max);
                *out = out.max(m);
            }
        }
    }
    if !have {
        // The static skin: the classic descending ramp, so the area still
        // reads as a visualizer without the audio source.
        for (i, v) in values.iter_mut().enumerate() {
            *v = 0.25 + 0.5 * ((bars - i) as f32 / bars as f32);
        }
    }
    // Widen the bands to the area: `per` cells each, a gap when there is
    // room for one.
    let per = (cols / bars.max(1)).max(1);
    let (thick, gap) = if per >= 3 { (per - 1, 1) } else { (per, 0) };
    let mut wide = Vec::with_capacity(cols);
    for v in &values {
        wide.extend(std::iter::repeat_n(*v, thick));
        wide.extend(std::iter::repeat_n(0.0, gap));
    }
    wide.truncate(cols);
    View::Bars {
        values: wide,
        gradient: GradientId::Audio,
        labels: None,
        peaks: None,
    }
}

/// Winamp's classic band count.
pub const BANDS: usize = 19;

/// The status tier: `▶ Artist — Title` over a two-cell posbar.
fn status_tier(w: &Winamp, cx: &RenderCx<'_>, now: &NowPlaying) -> View {
    let width = cx.inner.width;
    let head_room = width.saturating_sub(2);
    let text = window(&full_title(now), head_room, w.marquee_at(cx.now));
    let line: Line = vec![
        Span::new(role_for(now.status), format!("{} ", glyph(now.status))),
        Span::new(Role::Text, text),
    ];
    let mut children = vec![(Constraint::Len(1), View::Text(vec![line]))];
    if cx.inner.height >= 2 {
        children.push((Constraint::Len(1), posbar(now, cx, None)));
    }
    if cx.inner.height >= 3 {
        children.push((
            Constraint::Len(1),
            View::Text(vec![vec![Span::new(
                Role::TextMuted,
                clock(now.pos_at(cx.now)),
            )]]),
        ));
    }
    View::Stack {
        dir: Dir::V,
        children,
    }
}

/// The shade tier: the marquee, the clock and eight mini bars.
fn shade(w: &Winamp, cx: &RenderCx<'_>, now: &NowPlaying) -> View {
    let width = cx.inner.width;
    let elapsed = clock(now.pos_at(cx.now));
    let bars = if width >= 32 { 8usize } else { 0 };
    let head_room = width
        .saturating_sub(2)
        .saturating_sub(elapsed.chars().count() as u16 + 1)
        .saturating_sub(bars as u16 + 1);
    let mut line: Line = vec![
        Span::new(role_for(now.status), format!("{} ", glyph(now.status))),
        Span::new(
            Role::Text,
            window(&full_title(now), head_room, w.marquee_at(cx.now)),
        ),
        Span::new(Role::TextMuted, format!(" {elapsed}")),
    ];
    if now.len_us.is_none() {
        line.push(Span::new(Role::AccentSecondary, " ·"));
    }
    let mut children = vec![(Constraint::Len(1), View::Text(vec![line]))];
    if bars > 0 {
        children.push((Constraint::Len(1), vis(w, cx, bars)));
    }
    children.push((Constraint::Len(1), posbar(now, cx, None)));
    View::Stack {
        dir: Dir::V,
        children,
    }
}

/// `128 kbps · 48 kHz · stereo` from the audio source's sink, when it runs.
fn format_line(cx: &RenderCx<'_>) -> Option<Line> {
    let (_, sink) = cx.store.record(&audio::SINK)?;
    let mut line: Line = Vec::new();
    if sink.rate > 0 {
        line.push(Span::new(
            Role::TextMuted,
            format!("{} kHz", sink.rate / 1000),
        ));
    }
    if sink.channels > 0 {
        if !line.is_empty() {
            line.push(Span::new(Role::TextMuted, " · "));
        }
        line.push(Span::new(
            Role::TextMuted,
            if sink.channels >= 2 { "stereo" } else { "mono" },
        ));
    }
    (!line.is_empty()).then_some(line)
}

/// The transport row: what the player can do in `Text`, the rest ghosted.
fn transport(now: &NowPlaying) -> Line {
    let item = |on: bool, key: &'static str, label: &'static str| -> [Span; 2] {
        let role = if on { Role::Text } else { Role::TextGhost };
        [
            Span::new(role, format!("{key} ")),
            Span::new(if on { Role::TextMuted } else { Role::TextGhost }, label),
        ]
    };
    let c = now.caps;
    let mut line: Line = Vec::new();
    for (on, key, label) in [
        (c.prev, "z", "prev"),
        (c.play_pause, "x", "play"),
        (c.play_pause, "c", "pause"),
        (c.control, "v", "stop"),
        (c.next, "b", "next"),
    ] {
        if !line.is_empty() {
            line.push(Span::new(Role::TextGhost, " · "));
        }
        line.extend(item(on, key, label));
    }
    line
}

/// The volume slider as a gauge with its percentage.
fn volume(now: &NowPlaying) -> View {
    View::Gauge {
        label: Cow::Borrowed("vol"),
        value: now.volume as f32,
        gradient: GradientId::Load,
        text: Some(Cow::Owned(format!("{:.0}%", now.volume * 100.0))),
    }
}

/// The main tier, with the art column when `with_art`.
fn main_tier(w: &Winamp, cx: &RenderCx<'_>, now: &NowPlaying, with_art: bool) -> View {
    let art = with_art
        .then(|| cx.store.record(&media::ART).map(|(_, a)| a.clone()))
        .flatten()
        .filter(|a: &Art| a.is_valid() && a.track == now.track && w.options().art);
    // A cell is one column by two pixel rows, so a square cover wants
    // twice as many columns as rows — capped at a quarter of the tile and
    // 24 columns, or the art crowds out the player it belongs to.
    let art_cols = art
        .as_ref()
        .map(|_| {
            (cx.inner.height.saturating_sub(2) * 2)
                .min(cx.inner.width / 4)
                .min(24)
        })
        .filter(|c| *c >= 8)
        .unwrap_or(0);
    let art = art.filter(|_| art_cols > 0);
    let body_w = cx
        .inner
        .width
        .saturating_sub(art_cols + u16::from(art_cols > 0));
    let elapsed = clock(now.pos_at(cx.now));
    let head_room = body_w.saturating_sub(2);
    let mut head: Line = vec![
        Span::new(role_for(now.status), format!("{} ", glyph(now.status))),
        Span::bold(
            Role::Title,
            window(&full_title(now), head_room, w.marquee_at(cx.now)),
        ),
    ];
    if now.album.is_empty() {
        // Nothing: the album line is only worth a row when it exists.
    } else if body_w >= 40 {
        head.push(Span::new(Role::TextMuted, format!("  [{}]", now.album)));
    }
    let vis_cols = usize::from(body_w);
    let mut body: Vec<(Constraint, View)> = vec![
        (Constraint::Len(1), View::Text(vec![head])),
        (
            Constraint::Len(2),
            View::BigNumber {
                text: Cow::Owned(elapsed.clone()),
                role: role_for(now.status),
            },
        ),
        (Constraint::Fill(1), vis(w, cx, vis_cols)),
        (
            Constraint::Len(1),
            posbar(
                now,
                cx,
                now.len_us
                    .map(|len| format!("{elapsed} {}", remaining(now.pos_at(cx.now), len))),
            ),
        ),
        (Constraint::Len(1), volume(now)),
        (Constraint::Len(1), View::Text(vec![transport(now)])),
    ];
    if let Some(f) = format_line(cx)
        && cx.inner.height >= 12
    {
        body.insert(2, (Constraint::Len(1), View::Text(vec![f])));
    }
    let body = View::Stack {
        dir: Dir::V,
        children: body,
    };
    match art {
        Some(a) => {
            // A square cover is half as many rows as columns; the rest of
            // the column is left to the tile's background.
            let art_rows = (art_cols / 2).max(1).min(cx.inner.height);
            let column = View::Stack {
                dir: Dir::V,
                children: vec![
                    (
                        Constraint::Len(art_rows),
                        View::Custom {
                            paint: Box::new(ArtPainter::new(a)),
                            describe: Cow::Borrowed("album art"),
                        },
                    ),
                    (Constraint::Fill(1), View::Empty),
                ],
            };
            View::Stack {
                dir: Dir::H,
                children: vec![
                    (Constraint::Len(art_cols), column),
                    (Constraint::Len(1), View::Empty),
                    (Constraint::Fill(1), body),
                ],
            }
        }
        None => body,
    }
}

/// The zoom-only tier: the main body beside the playlist and the players.
fn full(w: &Winamp, cx: &RenderCx<'_>, now: &NowPlaying) -> View {
    let history = cx.store.record(&media::HISTORY).map(|(_, h)| h.clone());
    let players = cx.store.record(&media::PLAYERS).map(|(_, p)| p.clone());
    let rows: Vec<Vec<Line>> = history
        .map(|h| {
            h.tracks
                .iter()
                .rev()
                .map(|t| {
                    vec![
                        vec![Span::new(
                            if t.track == now.track {
                                Role::AccentPrimary
                            } else {
                                Role::Text
                            },
                            t.title.clone(),
                        )],
                        vec![Span::new(Role::TextMuted, t.artist.clone())],
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    let playlist = View::Stack {
        dir: Dir::V,
        children: vec![
            (
                Constraint::Len(1),
                View::Text(vec![vec![Span::bold(Role::Title, "playlist")]]),
            ),
            (
                Constraint::Fill(1),
                View::Table {
                    columns: vec![
                        Column {
                            title: "track".into(),
                            width: ColWidth::Elastic,
                            right: false,
                        },
                        Column {
                            title: "artist".into(),
                            width: ColWidth::Fixed(18),
                            right: false,
                        },
                    ],
                    rows,
                    selected: None,
                    sort: None,
                    scroll: w.scroll(),
                },
            ),
            (
                Constraint::Len(1),
                View::Text(vec![
                    players
                        .map(|p| {
                            let mut line: Line = vec![Span::new(Role::TextMuted, "players ")];
                            for (i, pl) in p.list.iter().enumerate() {
                                if i > 0 {
                                    line.push(Span::new(Role::TextMuted, " · "));
                                }
                                line.push(Span::new(
                                    if pl.is_current {
                                        Role::AccentPrimary
                                    } else {
                                        Role::TextMuted
                                    },
                                    if pl.identity.is_empty() {
                                        pl.bus.clone()
                                    } else {
                                        pl.identity.clone()
                                    },
                                ));
                            }
                            line.push(Span::new(Role::TextGhost, "   p cycles"));
                            line
                        })
                        .unwrap_or_else(|| vec![Span::new(Role::TextMuted, "players —")]),
                ]),
            ),
        ],
    };
    View::Stack {
        dir: Dir::H,
        children: vec![
            (Constraint::Fill(3), main_tier(w, cx, now, true)),
            (Constraint::Len(1), View::Empty),
            (Constraint::Fill(2), playlist),
        ],
    }
}
