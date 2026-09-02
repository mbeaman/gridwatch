//! The classic-skin marquee (brief arc 6 seam 5): one character every
//! 220 ms, the Winamp separator between repeats, and no scroll at all when
//! the text fits. Pure over `(text, width, now)` — `cx.now` is the run
//! clock, so a replayed journal and a headless shot scroll identically.

use gridwatch_store::Ts;
use unicode_width::UnicodeWidthChar;

/// A character every this many milliseconds (Winamp's own step).
pub const STEP_MS: u64 = 220;
/// What separates the end of the title from its repeat.
pub const SEPARATOR: &str = "  ***  ";

/// The visible window of `text` at `now`, `width` columns wide.
pub fn window(text: &str, width: u16, now: Ts) -> String {
    let width = usize::from(width);
    if width == 0 {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    if display_width(&chars) <= width {
        return text.to_string();
    }
    // The scrolling ring: the title, the separator, and back to the start.
    let ring: Vec<char> = text.chars().chain(SEPARATOR.chars()).collect();
    let steps = (now.0 / 1_000_000 / STEP_MS) as usize % ring.len().max(1);
    let mut out = String::with_capacity(width * 2);
    let mut used = 0usize;
    let mut i = 0usize;
    while used < width {
        let c = ring[(steps + i) % ring.len()];
        let w = c.width().unwrap_or(0);
        if w == 0 {
            i += 1;
            continue;
        }
        if used + w > width {
            // A wide glyph that would overflow: pad instead of splitting.
            out.push(' ');
            break;
        }
        out.push(c);
        used += w;
        i += 1;
    }
    out
}

fn display_width(chars: &[char]) -> usize {
    chars.iter().filter_map(|c| c.width()).sum()
}

/// `1:23` / `1:02:03` — the elapsed clock's text.
pub fn clock(us: i64) -> String {
    let secs = (us.max(0) / 1_000_000) as u64;
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The `-1:23` countdown Winamp shows when clicking the clock; used for the
/// remaining time beside the elapsed one.
pub fn remaining(pos_us: i64, len_us: i64) -> String {
    format!("-{}", clock((len_us - pos_us).max(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_never_scrolls() {
        for ms in [0u64, 220, 5_000] {
            let at = Ts(ms * 1_000_000);
            assert_eq!(window("hi", 10, at), "hi");
        }
        assert_eq!(window("hi", 0, Ts(0)), "");
    }

    #[test]
    fn a_long_title_steps_one_character_every_220ms_and_wraps() {
        let title = "Probably Stolen ROCK BOTTOM";
        let w = 10;
        let a = window(title, w, Ts(0));
        assert_eq!(a, "Probably S");
        assert_eq!(a.chars().count(), usize::from(w));
        // Under one step: unchanged.
        assert_eq!(window(title, w, Ts(219_000_000)), a);
        let b = window(title, w, Ts(220_000_000));
        assert_eq!(b, "robably St");
        // The separator appears between repeats, and the ring comes round.
        let ring_len = (title.chars().count() + SEPARATOR.chars().count()) as u64;
        let late = window(title, w, Ts(ring_len * STEP_MS * 1_000_000));
        assert_eq!(late, a, "one full turn is where it started");
        let mid = window(
            title,
            w,
            Ts((title.chars().count() as u64) * STEP_MS * 1_000_000),
        );
        assert!(mid.contains("***"), "the separator: {mid:?}");
    }

    #[test]
    fn wide_glyphs_are_never_split() {
        // Half-width katakana and a CJK title: the window stays inside the
        // column count whatever the mix.
        let title = "日本語のタイトルはとても長い場合があります";
        for w in 3..12u16 {
            for step in 0..8u64 {
                let out = window(title, w, Ts(step * STEP_MS * 1_000_000));
                let cols: usize = out.chars().filter_map(|c| c.width()).sum();
                assert!(cols <= usize::from(w), "{out:?} is {cols} wide, not {w}");
            }
        }
    }

    #[test]
    fn clocks_read_like_winamp() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(83_000_000), "1:23");
        assert_eq!(clock(3_723_000_000), "1:02:03");
        assert_eq!(clock(-5), "0:00");
        assert_eq!(remaining(83_000_000, 200_000_000), "-1:57");
        assert_eq!(remaining(300_000_000, 200_000_000), "-0:00");
    }
}
