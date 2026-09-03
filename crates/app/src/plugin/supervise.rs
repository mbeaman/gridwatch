//! Spawning and supervising one plugin (§4.7, D58 seam 7).
//!
//! The shape is a source's: a thread owns the child, reads its lines, and
//! reports what it learned. What is different is that everything the child
//! says is untrusted, so the reader validates before it believes, counts
//! strikes, and stops rather than restarting a plugin that cannot speak.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::proto::{self, Ask, Hello, Manifest, Refused, Says};

/// Three malformed messages and the plugin is stopped, not restarted.
pub const STRIKES: u32 = 3;

/// How many messages the host will read from one plugin per second.
///
/// This is the ceiling that makes a flooding plugin cost the *host* nothing
/// (P22). A plugin that writes faster than this is not read faster than this:
/// the reader stops for the rest of the second, its pipe fills, and the child
/// blocks in `write` — so neither process spins. Measured before it existed, a
/// plugin writing samples in a loop cost the host **62 % of a core** and
/// 578 000 wake-ups a second; 500 messages a second is far above what any real
/// plugin needs (the example publishes one) and far below what one can burn.
pub const MAX_MSGS_PER_SEC: u32 = 500;

/// A plugin over this share of one core, sustained for `RUNAWAY_WINDOW`, is
/// stopped (D58 seam 7). The rate budget already stops one that *writes* too
/// much; this is for one that simply spins. `RLIMIT_CPU` is the backstop
/// underneath, but its default of 600 s is ten minutes of a core beside a
/// game, which is not a limit anyone would call one.
pub const RUNAWAY_SHARE: f64 = 0.50;
pub const RUNAWAY_WINDOW: Duration = Duration::from_secs(10);

/// The runaway decision, as arithmetic: `Some(why)` when a plugin that has
/// used `used` of CPU over `elapsed` of wall time should be stopped. Split out
/// so it can be tested without burning a core for ten seconds.
pub fn runaway(elapsed: Duration, used: Duration) -> Option<String> {
    if elapsed < RUNAWAY_WINDOW {
        return None;
    }
    let share = used.as_secs_f64() / elapsed.as_secs_f64();
    if share < RUNAWAY_SHARE {
        return None;
    }
    Some(format!(
        "stopped: {:.0}% of a core for {} s (the ceiling is {:.0}%)",
        share * 100.0,
        elapsed.as_secs(),
        RUNAWAY_SHARE * 100.0
    ))
}

/// The inbound queue's depth (D58 seam 7). Full, it **drops the oldest**: a
/// reading nobody has read yet is worth less than the one after it, and a
/// queue that grows instead is how a plugin becomes the host's memory leak.
pub const QUEUE_DEPTH: usize = 64;

/// A bounded, drop-oldest queue between one plugin's reader thread and the
/// host. The reader is the only producer and the host thread the only
/// consumer, and neither is the render thread.
#[derive(Default)]
struct Inbox {
    queue: Mutex<Queued>,
    ready: Condvar,
}

#[derive(Default)]
struct Queued {
    reports: VecDeque<Report>,
    dropped: u64,
    closed: bool,
}

impl Inbox {
    /// Push, dropping the oldest if the queue is full. Returns false once the
    /// consumer is gone, which is the reader's signal to stop.
    fn push(&self, report: Report) -> bool {
        let Ok(mut q) = self.queue.lock() else {
            return false;
        };
        if q.closed {
            return false;
        }
        if q.reports.len() >= QUEUE_DEPTH {
            q.reports.pop_front();
            q.dropped += 1;
        }
        q.reports.push_back(report);
        self.ready.notify_all();
        true
    }

    fn drain(&self) -> Vec<Report> {
        match self.queue.lock() {
            Ok(mut q) => q.reports.drain(..).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn next(&self, for_: Duration) -> Option<Report> {
        let deadline = Instant::now() + for_;
        let mut q = self.queue.lock().ok()?;
        loop {
            if let Some(r) = q.reports.pop_front() {
                return Some(r);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            let (next, timeout) = self.ready.wait_timeout(q, left).ok()?;
            q = next;
            if timeout.timed_out() && q.reports.is_empty() {
                return None;
            }
        }
    }

    fn dropped(&self) -> u64 {
        self.queue.lock().map(|q| q.dropped).unwrap_or(0)
    }

    fn close(&self) {
        if let Ok(mut q) = self.queue.lock() {
            q.closed = true;
            q.reports.clear();
            self.ready.notify_all();
        }
    }
}

/// How long the host waits for the manifest before giving up on a plugin
/// that never speaks.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// Restart backoff, capped. A plugin that keeps dying is retried more and
/// more slowly, and never in a tight loop.
pub const BACKOFF: &[Duration] = &[
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(30),
];

/// What a plugin instance is configured to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginConfig {
    /// The program and its arguments. **Not** a shell string: the first
    /// element is the program, the rest are arguments, verbatim.
    pub argv: Vec<String>,
    /// The id this plugin's keys and kind are namespaced under.
    pub id: String,
    /// Memory ceiling for the child, in MiB.
    pub rss_mb: u64,
    /// CPU seconds the child may use before the kernel stops it.
    pub cpu_secs: u64,
}

impl PluginConfig {
    pub fn new(id: &str, argv: Vec<String>) -> PluginConfig {
        PluginConfig {
            argv,
            id: id.to_string(),
            rss_mb: 256,
            cpu_secs: 600,
        }
    }
}

/// What the reader thread tells the host.
#[derive(Clone, Debug, PartialEq)]
pub enum Report {
    /// The manifest, once, and only if it is placeable.
    Ready(Box<Manifest>),
    /// A metric reading, with the plugin's id already prefixed.
    Sample {
        key: String,
        label: Option<proto::WireLabel>,
        at: Option<i64>,
        value: f64,
    },
    /// A view tree for an instance, as JSON (the host deserialises it into
    /// the same `View` a built-in returns).
    View {
        instance: String,
        tree: serde_json::Value,
    },
    Command(proto::Cmd),
    Status {
        state: proto::State,
        reason: Option<String>,
        hint: Option<String>,
    },
    Log {
        level: Option<String>,
        text: String,
    },
    /// A line the host refused, and which strike it was.
    Refused {
        why: String,
        strike: u32,
    },
    /// The plugin is finished: three strikes, a spawn failure, or its own
    /// exit. The host shows this as `Unavailable` with the reason.
    Stopped(String),
}

/// A running (or failed) plugin.
pub struct Plugin {
    pub config: PluginConfig,
    inbox: Arc<Inbox>,
    stdin: Option<ChildStdin>,
    child: Option<Child>,
    reader: Option<std::thread::JoinHandle<()>>,
    started: Instant,
}

// The one `unsafe` this module needs: `pre_exec` is unsafe by signature
// because the closure runs between fork and exec, where only
// async-signal-safe calls are allowed. `setrlimit(2)` is one of them.
#[allow(unsafe_code)]
/// Set the child's own limits before it runs. `pre_exec` runs in the child
/// between fork and exec, where only async-signal-safe calls are allowed —
/// `setrlimit` is one.
#[cfg(unix)]
fn with_limits(cmd: &mut Command, rss_mb: u64, cpu_secs: u64) {
    use std::os::unix::process::CommandExt;
    let as_bytes = rss_mb.saturating_mul(1024 * 1024);
    // SAFETY: `pre_exec` requires the closure to be async-signal-safe.
    // `setrlimit(2)` is on the POSIX list; it allocates nothing, takes no
    // locks, and touches only this (freshly forked) process.
    unsafe {
        cmd.pre_exec(move || {
            let lim = |resource, value| {
                let rl = libc::rlimit {
                    rlim_cur: value,
                    rlim_max: value,
                };
                libc::setrlimit(resource, &rl)
            };
            // A plugin that asks for more address space than this gets a
            // failed allocation, which is its problem to report, not the
            // host's to absorb.
            let _ = lim(libc::RLIMIT_AS, as_bytes);
            let _ = lim(libc::RLIMIT_CPU, cpu_secs);
            Ok(())
        });
    }
}

impl Plugin {
    /// Spawn the child and start reading it. Never runs a shell.
    pub fn spawn(config: PluginConfig, hello: Hello) -> Plugin {
        let started = Instant::now();
        let inbox = Arc::new(Inbox::default());
        let Some((program, args)) = config.argv.split_first() else {
            inbox.push(Report::Stopped("no command configured".into()));
            return Plugin {
                config,
                inbox,
                stdin: None,
                child: None,
                reader: None,
                started,
            };
        };
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // A plugin's stderr belongs in the log, never on the screen.
            .stderr(Stdio::null())
            // Nothing of the host's environment is a plugin's business
            // except what it needs to run.
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("GRIDWATCH_PLUGIN_CONTRACT", proto::CONTRACT.to_string());
        #[cfg(unix)]
        with_limits(&mut cmd, config.rss_mb, config.cpu_secs);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                inbox.push(Report::Stopped(format!("{program}: {e}")));
                return Plugin {
                    config,
                    inbox,
                    stdin: None,
                    child: None,
                    reader: None,
                    started,
                };
            }
        };
        let mut stdin = child.stdin.take();
        // The hello goes first, before anything is read. A plugin that
        // never reads its input is unusual but not broken — it may be a
        // pure source that only writes — so a failed write here is a log
        // line, not a death: what the plugin *says* decides its fate.
        if let Some(w) = stdin.as_mut() {
            let line = serde_json::to_string(&hello).unwrap_or_default();
            if writeln!(w, "{line}").is_err() || w.flush().is_err() {
                tracing::debug!(id = %config.id, "the plugin did not read its hello");
            }
        }
        let stdout = child.stdout.take();
        let id = config.id.clone();
        let to_host = Arc::clone(&inbox);
        let reader = stdout.map(|out| {
            std::thread::Builder::new()
                .name(format!("gw-plugin-{id}"))
                .spawn(move || read_lines(out, &id, &to_host))
                .expect("spawn a reader thread")
        });
        Plugin {
            config,
            inbox,
            stdin,
            child: Some(child),
            reader,
            started,
        }
    }

    /// Everything the plugin has said since the last call. Never blocks.
    pub fn drain(&self) -> Vec<Report> {
        self.inbox.drain()
    }

    /// Wait for the next report, up to `for_`. Used by the handshake and
    /// by tests; the frame loop uses `drain`.
    pub fn next_report(&self, for_: Duration) -> Option<Report> {
        self.inbox.next(for_)
    }

    /// How many reports the queue has dropped to stay bounded — a number the
    /// host shows rather than hides (D58 seam 7).
    pub fn dropped(&self) -> u64 {
        self.inbox.dropped()
    }

    /// Ask the plugin for something. A closed pipe is not an error worth
    /// shouting about — the reader will report the exit.
    pub fn ask(&mut self, ask: &Ask) -> bool {
        let Some(w) = self.stdin.as_mut() else {
            return false;
        };
        let Ok(line) = serde_json::to_string(ask) else {
            return false;
        };
        writeln!(w, "{line}").is_ok() && w.flush().is_ok()
    }

    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }

    /// The child's pid, while it is running.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.id())
    }

    /// CPU the child has used, from `/proc/<pid>/stat`'s `utime + stime`
    /// (fields 14 and 15, after the comm field — which can itself contain
    /// spaces and brackets, so the split is on the last `)`). `None` when the
    /// process is gone or the file will not parse; the supervisor treats that
    /// as "no reading", never as "zero".
    pub fn cpu_used(&self) -> Option<Duration> {
        let pid = self.pid()?;
        let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let after_comm = text.rsplit_once(')')?.1;
        let mut fields = after_comm.split_whitespace();
        // The first field here is `state`, so utime is the 12th after it.
        let utime: u64 = fields.nth(11)?.parse().ok()?;
        let stime: u64 = fields.next()?.parse().ok()?;
        let hz = 100; // CONFIG_HZ on every kernel this runs on; `getconf CLK_TCK`
        Some(Duration::from_millis((utime + stime) * 1000 / hz))
    }

    /// How long to wait before the nth restart.
    pub fn backoff(restarts: usize) -> Duration {
        BACKOFF[restarts.min(BACKOFF.len() - 1)]
    }

    /// Stop the child and wait for the reader. Called on drop, and by the
    /// host when a plugin strikes out.
    pub fn stop(&mut self) {
        self.stdin = None;
        // Close before the kill: a reader blocked on `push` learns the
        // consumer is gone, and one blocked on `read_line` wakes when the
        // child's stdout closes.
        self.inbox.close();
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(h) = self.reader.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The reader thread: one line at a time, validated, counted, reported.
fn read_lines(out: std::process::ChildStdout, id: &str, tx: &Inbox) {
    let mut reader = BufReader::new(out);
    let mut strikes = 0u32;
    let mut line = String::new();
    let mut manifest_seen = false;
    // The rate budget (P22): messages read in the current second, and when
    // that second began.
    let mut window = Instant::now();
    let mut read_this_second = 0u32;
    loop {
        // Do not read faster than the budget. Stopping here is what makes a
        // flooding plugin free: the pipe fills, the child blocks in `write`,
        // and neither process spins. It is deliberately *before* the read, so
        // no line is parsed above the budget either.
        read_this_second += 1;
        if read_this_second > MAX_MSGS_PER_SEC {
            let left = Duration::from_secs(1).saturating_sub(window.elapsed());
            if !left.is_zero() {
                tracing::debug!(
                    target: "gridwatch::plugin",
                    "{id}: over {MAX_MSGS_PER_SEC} messages/s — not reading for {} ms",
                    left.as_millis()
                );
                std::thread::sleep(left);
            }
        }
        if window.elapsed() >= Duration::from_secs(1) {
            window = Instant::now();
            read_this_second = 1;
        }
        line.clear();
        // A bounded read: a plugin that never writes a newline cannot make
        // the host grow a string until the machine notices.
        let mut limited = Read::take(&mut reader, (proto::MAX_LINE + 1) as u64);
        match limited.read_line(&mut line) {
            Ok(0) => {
                tx.push(Report::Stopped("the plugin exited".into()));
                return;
            }
            Ok(_) => {}
            Err(e) => {
                tx.push(Report::Stopped(format!("read: {e}")));
                return;
            }
        }
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            continue;
        }
        let report = match proto::parse(trimmed) {
            Ok(Says::Manifest { manifest }) => match proto::check_manifest(&manifest) {
                Ok(()) if manifest_seen => Err(Refused::BadManifest(
                    "a second manifest; a plugin declares itself once".into(),
                )),
                Ok(()) => {
                    manifest_seen = true;
                    Ok(Report::Ready(manifest))
                }
                Err(e) => Err(e),
            },
            Ok(Says::Sample {
                key,
                label,
                at,
                value,
            }) => {
                // A plugin's keys live under its own id, so it can never
                // publish over a built-in metric.
                if !value.is_finite() {
                    Err(Refused::Malformed(format!("{key}: a value of {value}")))
                } else {
                    Ok(Report::Sample {
                        key: format!(
                            "{id}.{}",
                            key.split_once('.').map(|(_, m)| m).unwrap_or(&key)
                        ),
                        label,
                        at,
                        value,
                    })
                }
            }
            Ok(Says::View { instance, tree }) => Ok(Report::View { instance, tree }),
            Ok(Says::Command { command }) => Ok(Report::Command(command)),
            Ok(Says::Status {
                state,
                reason,
                hint,
            }) => Ok(Report::Status {
                state,
                reason,
                hint,
            }),
            Ok(Says::Log { level, text }) => Ok(Report::Log { level, text }),
            Err(e) => Err(e),
        };
        match report {
            Ok(r) => {
                if !tx.push(r) {
                    return; // the host is gone
                }
            }
            Err(why) => {
                strikes += 1;
                if !tx.push(Report::Refused {
                    why: why.to_string(),
                    strike: strikes,
                }) {
                    return;
                }
                if strikes >= STRIKES {
                    tx.push(Report::Stopped(format!(
                        "{STRIKES} malformed messages; the last was {why}"
                    )));
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello() -> Hello {
        Hello::new(vec!["Procfs".into()], vec!["cpu.total_pct".into()])
    }

    /// A plugin written as a shell script would be the easy way to test
    /// this — and this host never runs a shell, so the fixture is `cat`
    /// and friends, driven through real argv.
    fn echo_plugin(lines: &[&str]) -> PluginConfig {
        // `printf` writes exactly what it is given and exits; no shell is
        // involved because argv is passed as a vector.
        let mut argv = vec!["printf".to_string(), "%s\\n".to_string()];
        for l in lines {
            argv.push((*l).to_string());
        }
        PluginConfig::new("test", argv)
    }

    fn drain_until_stopped(p: &Plugin) -> Vec<Report> {
        let mut out = Vec::new();
        while let Some(r) = p.next_report(Duration::from_secs(3)) {
            let stop = matches!(r, Report::Stopped(_));
            out.push(r);
            if stop {
                break;
            }
        }
        out
    }

    #[test]
    fn a_plugin_that_speaks_is_understood() {
        let manifest = r#"{"kind":"manifest","manifest":{"kind":"weather","name":"weather","contract":1,"tiers":[{"name":"badge","min":{"w":8,"h":3}}],"produces":[{"key":"weather.temp_c"}]}}"#;
        let p = Plugin::spawn(
            echo_plugin(&[
                manifest,
                r#"{"kind":"status","state":"ok","reason":"reading"}"#,
                r#"{"kind":"sample","key":"weather.temp_c","value":14.5}"#,
                r#"{"kind":"log","level":"debug","text":"hello"}"#,
            ]),
            hello(),
        );
        let reports = drain_until_stopped(&p);
        assert!(
            matches!(reports.first(), Some(Report::Ready(m)) if m.kind == "weather"),
            "{reports:?}"
        );
        assert!(reports.iter().any(|r| matches!(
            r,
            Report::Status {
                state: proto::State::Ok,
                ..
            }
        )));
        // The key is namespaced under the plugin's id: a plugin cannot
        // publish over `cpu.total_pct` however it names its metric.
        let sample = reports
            .iter()
            .find_map(|r| match r {
                Report::Sample { key, value, .. } => Some((key.clone(), *value)),
                _ => None,
            })
            .expect("a sample");
        assert_eq!(sample.0, "test.temp_c");
        assert_eq!(sample.1, 14.5);
        assert!(reports.iter().any(|r| matches!(r, Report::Log { .. })));
        assert!(matches!(reports.last(), Some(Report::Stopped(_))));
    }

    /// Three malformed messages stop it. The fourth line would have been
    /// perfectly good, and is never read.
    #[test]
    fn three_strikes_stop_a_plugin_that_cannot_speak() {
        let p = Plugin::spawn(
            echo_plugin(&[
                "not json at all",
                r#"{"kind":"sample","key":"BAD KEY","value":1}"#,
                r#"{"kind":"nonsense"}"#,
                r#"{"kind":"status","state":"ok"}"#,
            ]),
            hello(),
        );
        let reports = drain_until_stopped(&p);
        let strikes: Vec<u32> = reports
            .iter()
            .filter_map(|r| match r {
                Report::Refused { strike, .. } => Some(*strike),
                _ => None,
            })
            .collect();
        assert_eq!(strikes, vec![1, 2, 3]);
        let Some(Report::Stopped(why)) = reports.last() else {
            panic!("{reports:?}");
        };
        assert!(why.contains("3 malformed"), "{why}");
        assert!(
            !reports.iter().any(|r| matches!(r, Report::Status { .. })),
            "the fourth line was never read"
        );
    }

    /// A manifest the host cannot place is a strike, not a tile.
    #[test]
    fn an_unplaceable_manifest_is_refused_with_its_reason() {
        let m = r#"{"kind":"manifest","manifest":{"kind":"big","name":"big","contract":1,"tiers":[{"name":"only","min":{"w":80,"h":24}}]}}"#;
        let p = Plugin::spawn(echo_plugin(&[m]), hello());
        let reports = drain_until_stopped(&p);
        let Some(Report::Refused { why, strike }) = reports.first() else {
            panic!("{reports:?}");
        };
        assert_eq!(*strike, 1);
        assert!(why.contains("does not fit"), "{why}");
    }

    /// A program that is not there is a stopped plugin with a reason, not
    /// a panic and not a silent absence.
    #[test]
    fn a_missing_program_says_so() {
        let p = Plugin::spawn(
            PluginConfig::new("ghost", vec!["/nonexistent/plugin".into()]),
            hello(),
        );
        let Some(Report::Stopped(why)) = p.next_report(Duration::from_secs(2)) else {
            panic!("expected a Stopped report");
        };
        assert!(why.contains("/nonexistent/plugin"), "{why}");
        // And an empty command line, which a config could produce.
        let p = Plugin::spawn(PluginConfig::new("empty", Vec::new()), hello());
        assert!(matches!(
            p.next_report(Duration::from_secs(2)),
            Some(Report::Stopped(_))
        ));
    }

    /// A plugin that says nothing at all is not a hang: the host reads
    /// nothing, and stops waiting.
    #[test]
    fn a_silent_plugin_does_not_hang_the_host() {
        let p = Plugin::spawn(
            PluginConfig::new("quiet", vec!["sleep".into(), "30".into()]),
            hello(),
        );
        let t0 = Instant::now();
        assert!(p.next_report(Duration::from_millis(300)).is_none());
        assert!(t0.elapsed() < Duration::from_secs(2));
        drop(p); // kills and joins
    }

    /// The shipped example, end to end through the real host: it
    /// declares itself, publishes a reading, and answers a render with a
    /// view tree. This is the test a plugin author's own plugin should
    /// pass before they file a bug.
    #[test]
    fn the_example_plugin_declares_itself_and_draws() {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../plugins/examples/weather.py");
        assert!(script.is_file(), "the example is shipped");
        let mut p = Plugin::spawn(
            PluginConfig::new(
                "weather",
                vec!["python3".into(), script.to_string_lossy().to_string()],
            ),
            hello(),
        );
        // The manifest, and it is placeable.
        let Some(Report::Ready(manifest)) = p.next_report(Duration::from_secs(5)) else {
            panic!("no manifest from the example plugin");
        };
        assert_eq!(manifest.kind, "weather");
        assert_eq!(manifest.tiers.len(), 2);
        assert_eq!(manifest.produces[0].key, "weather.temp_c");
        proto::check_manifest(&manifest).expect("placeable");

        // A status and a first reading, without being asked.
        let mut sample = None;
        let mut status = None;
        for _ in 0..4 {
            match p.next_report(Duration::from_secs(5)) {
                Some(Report::Sample { key, value, .. }) => sample = Some((key, value)),
                Some(Report::Status { state, .. }) => status = Some(state),
                Some(_) => {}
                None => break,
            }
            if sample.is_some() && status.is_some() {
                break;
            }
        }
        let (key, value) = sample.expect("a reading");
        assert_eq!(key, "weather.temp_c", "namespaced under the plugin id");
        assert!(
            (-90.0..60.0).contains(&value),
            "a plausible temperature: {value}"
        );
        assert!(status.is_some(), "it says how it is doing");

        // And it draws when asked, returning a tree the host can parse.
        assert!(p.ask(&Ask::Render {
            instance: "outside".into(),
            tier: 1,
            inner: proto::Size { w: 30, h: 8 },
            now: 1,
            focused: false,
            captured: false,
        }));
        let mut tree = None;
        for _ in 0..4 {
            if let Some(Report::View { instance, tree: t }) = p.next_report(Duration::from_secs(5))
            {
                assert_eq!(instance, "outside");
                tree = Some(t);
                break;
            }
        }
        let tree = tree.expect("a view tree");
        // It is a stack of text, with no colour of its own anywhere.
        let text = tree.to_string();
        assert!(text.contains("stack"), "{text}");
        assert!(text.contains("°C"), "{text}");
        assert!(
            !text.contains('#') && !text.to_lowercase().contains("rgb"),
            "a plugin may not choose colours: {text}"
        );
    }

    /// The backoff never runs a restart loop hot.
    #[test]
    fn the_backoff_grows_and_caps() {
        assert_eq!(Plugin::backoff(0), Duration::from_secs(1));
        assert_eq!(Plugin::backoff(3), Duration::from_secs(8));
        assert_eq!(Plugin::backoff(4), Duration::from_secs(30));
        assert_eq!(Plugin::backoff(99), Duration::from_secs(30));
    }

    /// A line longer than the cap is refused, and the reader keeps its
    /// footing rather than growing a string forever.
    #[test]
    fn an_enormous_line_is_a_strike_not_a_memory_leak() {
        // A megabyte will not fit in an argv, so this plugin generates it
        // — still no shell: a program and its arguments.
        let p = Plugin::spawn(
            PluginConfig::new(
                "huge",
                vec![
                    "python3".into(),
                    "-c".into(),
                    format!(
                        "print('{{\"kind\":\"log\",\"text\":\"' + 'x'*{} + '\"}}')\nprint('also bad')\nprint('still bad')",
                        proto::MAX_LINE
                    ),
                ],
            ),
            hello(),
        );
        let reports = drain_until_stopped(&p);
        assert!(
            reports.iter().any(|r| matches!(r, Report::Refused { .. })),
            "{:?}",
            reports.len()
        );
        assert!(matches!(reports.last(), Some(Report::Stopped(_))));
    }
}
