//! Hot reload's watcher (§9, brief arc 3 seam 8): a `gw-watch` std thread
//! `stat()`s `config.toml`, `layout.toml` and the active theme file once per
//! second — no `notify`, so an editor's rename-over-write is just another
//! mtime — and sends `ControlMsg::Reload` on an mtime or size change. The
//! shell does the parsing on the render thread (the files are small). A save
//! made by a future edit mode registers its content hash first; the watcher
//! skips one change whose bytes hash to it (the slot is spec'd in §9 and
//! unused until arc 4).

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, SystemTime};

use gridwatch_store::{ControlMsg, Reload, ReloadKind};

/// One watched file and what a change to it means.
#[derive(Clone, Debug)]
pub struct Watched {
    pub kind: ReloadKind,
    pub path: PathBuf,
}

/// The stat signature the watcher compares: absent, or (mtime, size).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp(Option<(SystemTime, u64)>);

pub fn stamp(path: &Path) -> Stamp {
    Stamp(
        std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok().map(|t| (t, m.len()))),
    )
}

/// The hash the ignore slot compares against (§9 — edit-mode saves).
pub fn content_hash(bytes: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// The watcher's poll period (§9: once per second — P5 counts it as 1 wake/s).
pub const PERIOD: Duration = Duration::from_secs(1);

/// The ignore slots, one per `ReloadKind`, as atomics (§11: no lock is ever
/// taken on the render thread, which is where edit mode will register a
/// save). `0` is "empty"; a hash that happens to be 0 is nudged to 1.
type IgnoreSlots = Arc<[AtomicU64; 3]>;

pub struct WatchHandle {
    stop: Arc<AtomicBool>,
    ignore: IgnoreSlots,
    /// Replaces the watched theme files (`kind == Theme`) — the theme the
    /// shell reloads moves with `t` and with a `config.toml` edit.
    theme_files: Sender<Vec<Watched>>,
    join: Option<std::thread::JoinHandle<()>>,
}

fn kind_key(k: ReloadKind) -> u8 {
    match k {
        ReloadKind::Config => 0,
        ReloadKind::Layout => 1,
        ReloadKind::Theme => 2,
    }
}

impl WatchHandle {
    /// Register a save's content hash: the next change to a file of `kind`
    /// whose bytes hash to it is not reported (edit mode's own write).
    pub fn ignore_next(&self, kind: ReloadKind, hash: u64) {
        self.ignore[usize::from(kind_key(kind))].store(hash.max(1), Ordering::Release);
    }

    /// A sender the shell keeps: send the new theme file list whenever the
    /// reload target changes (`watch::theme_files(theme_ref)`).
    pub fn theme_files_sender(&self) -> Sender<Vec<Watched>> {
        self.theme_files.clone()
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// The decision the poll makes for one file, factored out so it is testable
/// without a thread: a changed stamp reports, unless the ignore slot matches
/// the new content (then the slot is consumed and nothing is reported).
pub fn judge(
    prev: Stamp,
    now: Stamp,
    ignore: Option<u64>,
    read: impl FnOnce() -> Option<u64>,
) -> (bool, bool) {
    if prev == now {
        return (false, false);
    }
    if let Some(h) = ignore
        && read() == Some(h)
    {
        return (false, true);
    }
    (true, false)
}

/// Start the watcher over `files`; the initial stamps are taken here so only
/// later changes report.
pub fn spawn(files: Vec<Watched>, control: Sender<ControlMsg>) -> WatchHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let ignore: IgnoreSlots = Arc::new([AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)]);
    let (theme_tx, theme_rx): (Sender<Vec<Watched>>, Receiver<Vec<Watched>>) = channel();
    let stop_t = stop.clone();
    let ignore_t = ignore.clone();
    let join = std::thread::Builder::new()
        .name("gw-watch".into())
        .spawn(move || {
            let mut stamps: Vec<(Watched, Stamp)> = files
                .into_iter()
                .map(|w| (stamp(&w.path), w))
                .map(|(s, w)| (w, s))
                .collect();
            loop {
                // One wake per second; the stop flag is checked every 250 ms
                // so shutdown never waits out the period.
                for _ in 0..4 {
                    if stop_t.load(Ordering::Acquire) {
                        return;
                    }
                    std::thread::sleep(PERIOD / 4);
                }
                // The theme moved (`t`, a config edit): swap the Theme-kind
                // entries, stamped now so the new file's state is the baseline.
                while let Ok(new) = theme_rx.try_recv() {
                    stamps.retain(|(w, _)| w.kind != ReloadKind::Theme);
                    stamps.extend(new.into_iter().map(|w| {
                        let s = stamp(&w.path);
                        (w, s)
                    }));
                }
                for (w, prev) in stamps.iter_mut() {
                    let now = stamp(&w.path);
                    let slot = &ignore_t[usize::from(kind_key(w.kind))];
                    let pending = match slot.load(Ordering::Acquire) {
                        0 => None,
                        h => Some(h),
                    };
                    let (report, consumed) = judge(*prev, now, pending, || {
                        std::fs::read(&w.path).ok().map(|b| content_hash(&b).max(1))
                    });
                    *prev = now;
                    if consumed {
                        slot.store(0, Ordering::Release);
                    }
                    if report
                        && control
                            .send(ControlMsg::Reload(Reload { kind: w.kind }))
                            .is_err()
                    {
                        return; // the render thread is gone
                    }
                }
            }
        })
        .expect("spawn gw-watch");
    WatchHandle {
        stop,
        ignore,
        theme_files: theme_tx,
        join: Some(join),
    }
}

/// The theme files a run should watch: the theme path itself when the theme
/// is a file (built-ins are embedded — nothing to watch), plus the sibling it
/// `inherits`, when that is a file too.
pub fn theme_files(theme_ref: &str) -> Vec<Watched> {
    if !theme_ref.ends_with(".toml") {
        return Vec::new();
    }
    let path = PathBuf::from(theme_ref);
    let mut out = vec![Watched {
        kind: ReloadKind::Theme,
        path: path.clone(),
    }];
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(file) = gridwatch_ui::theme::load_theme_file(&text)
        && let Some(parent) = file.meta.inherits
        && gridwatch_ui::theme::builtin(&parent).is_none()
    {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let sibling = if parent.ends_with(".toml") {
            dir.join(parent)
        } else {
            dir.join(format!("{parent}.toml"))
        };
        out.push(Watched {
            kind: ReloadKind::Theme,
            path: sibling,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judge_reports_a_change_and_skips_one_ignored_hash() {
        let a = Stamp(Some((SystemTime::UNIX_EPOCH, 10)));
        let b = Stamp(Some((SystemTime::UNIX_EPOCH + Duration::from_secs(1), 10)));
        assert_eq!(judge(a, a, None, || None), (false, false));
        assert_eq!(judge(a, b, None, || None), (true, false));
        let h = content_hash(b"saved by edit mode");
        assert_eq!(judge(a, b, Some(h), || Some(h)), (false, true));
        // A different write while a hash is pending still reports.
        assert_eq!(judge(a, b, Some(h), || Some(h + 1)), (true, false));
        // Appearing and disappearing are changes too.
        assert_eq!(judge(Stamp(None), a, None, || None), (true, false));
        assert_eq!(judge(a, Stamp(None), None, || None), (true, false));
    }

    #[test]
    fn theme_files_are_empty_for_builtins() {
        assert!(theme_files("retrowave").is_empty());
        assert_eq!(theme_files("/nonexistent/x.toml").len(), 1);
    }
}
