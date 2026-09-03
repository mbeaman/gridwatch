//! D46 layer C: the binary run the way a person runs it — under a real pty
//! (util-linux `script`), and once with no tty at all. Every assertion is
//! about what the user would have seen in the terminal, not about a buffer.
//!
//! `script` ships with util-linux on every Linux box and the CI runner; when
//! it is missing the pty tests print why and pass, so the suite still runs on
//! a stripped container — but that case is visible in the output, not silent.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_gridwatch")
}

fn have_script() -> bool {
    Command::new("script")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// A private XDG tree per test: config comes from the embedded defaults, the
/// log lands somewhere we can read, and nothing touches the developer's own
/// `~/.config/gridwatch`.
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let root = std::env::temp_dir().join(format!("gridwatch-pty-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::create_dir_all(root.join("state")).unwrap();
        Sandbox { root }
    }

    fn typescript(&self) -> PathBuf {
        self.root.join("typescript")
    }

    fn log(&self) -> PathBuf {
        self.root.join("state/gridwatch/gridwatch.log")
    }

    fn env(&self, cmd: &mut Command, tag: &str) {
        cmd.env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("GRIDWATCH_PTY_TEST", tag)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor");
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The pty session: `script` owns the pty, gridwatch runs inside it, we hold
/// its stdin (keystrokes) and read the typescript (everything it drew).
struct Session {
    child: Child,
    sandbox: Sandbox,
    tag: String,
}

impl Session {
    fn start(tag: &str, rows: u16, cols: u16, args: &str) -> Session {
        Session::start_with_env(tag, rows, cols, args, &[])
    }

    /// The same, with extra environment for the run. Arc 8a's action tests
    /// use it to fence the executor to a pid nothing spawned, so a
    /// confirmed action could not touch anything even if one slipped
    /// through (D58).
    fn start_with_env(
        tag: &str,
        rows: u16,
        cols: u16,
        args: &str,
        env: &[(&str, &str)],
    ) -> Session {
        Session::start_in_with_env(tag, Sandbox::new(tag), rows, cols, args, env)
    }

    /// The same over a sandbox the test already filled — arc 8b's plugin rows
    /// have to write `config.toml` *before* the run, and `Sandbox::new` wipes
    /// the tree it creates.
    fn start_in(tag: &str, sandbox: Sandbox, rows: u16, cols: u16, args: &str) -> Session {
        Session::start_in_with_env(tag, sandbox, rows, cols, args, &[])
    }

    fn start_in_with_env(
        tag: &str,
        sandbox: Sandbox,
        rows: u16,
        cols: u16,
        args: &str,
        env: &[(&str, &str)],
    ) -> Session {
        let inner = format!("stty rows {rows} cols {cols}; exec {} {args}", bin());
        let mut cmd = Command::new("script");
        cmd.args(["-q", "-f", "-e", "-c", &inner])
            .arg(sandbox.typescript())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        sandbox.env(&mut cmd, tag);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = cmd.spawn().expect("spawn script");
        Session {
            child,
            sandbox,
            tag: tag.to_string(),
        }
    }

    /// What the terminal has received so far, with SGR/CSI stripped.
    fn screen(&self) -> String {
        let raw = std::fs::read(self.sandbox.typescript()).unwrap_or_default();
        strip_escapes(&String::from_utf8_lossy(&raw))
    }

    fn raw(&self) -> Vec<u8> {
        std::fs::read(self.sandbox.typescript()).unwrap_or_default()
    }

    /// Poll the screen until `pred` holds or `within` elapses.
    fn wait_for(&self, within: Duration, pred: impl Fn(&str) -> bool) -> Option<String> {
        let t = Instant::now();
        while t.elapsed() < within {
            let s = self.screen();
            if pred(&s) {
                return Some(s);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    fn keys(&mut self, s: &str) {
        let stdin = self.child.stdin.as_mut().expect("stdin");
        let _ = stdin.write_all(s.as_bytes());
        let _ = stdin.flush();
    }

    /// The pty the *gridwatch* process (not `script`) is attached to, found by
    /// the tag in its environment — several sessions run in parallel.
    fn pts(&self) -> Option<PathBuf> {
        let want = format!("GRIDWATCH_PTY_TEST={}", self.tag);
        for entry in std::fs::read_dir("/proc").ok()?.flatten() {
            let pid = entry.file_name();
            let Some(pid) = pid.to_str() else { continue };
            if !pid.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let comm = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
            if comm.trim() != "gridwatch" {
                continue;
            }
            let environ = std::fs::read(entry.path().join("environ")).unwrap_or_default();
            if environ.split(|b| *b == 0).any(|kv| kv == want.as_bytes()) {
                return std::fs::read_link(entry.path().join("fd/0")).ok();
            }
        }
        None
    }

    fn resize(&self, rows: u16, cols: u16) {
        let pts = self.pts().expect("find gridwatch's pty");
        let ok = Command::new("stty")
            .args(["-F"])
            .arg(&pts)
            .args(["rows", &rows.to_string(), "cols", &cols.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "stty on {}", pts.display());
    }

    fn finish(mut self) -> (i32, Vec<u8>, String) {
        let t = Instant::now();
        let code = loop {
            if let Ok(Some(st)) = self.child.try_wait() {
                break st.code().unwrap_or(-1);
            }
            if t.elapsed() > Duration::from_secs(5) {
                let _ = self.child.kill();
                break -9;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let raw = self.raw();
        let log = std::fs::read_to_string(self.sandbox.log()).unwrap_or_default();
        (code, raw, log)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Drop CSI/OSC sequences so assertions read the characters a person saw.
fn strip_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                let mut prev = '\0';
                for c in chars.by_ref() {
                    if c == '\x07' || (prev == '\x1b' && c == '\\') {
                        break;
                    }
                    prev = c;
                }
            }
            _ => {}
        }
    }
    out
}

fn skip(name: &str) -> bool {
    if have_script() {
        return false;
    }
    eprintln!("{name}: util-linux `script` not found — pty test skipped (visible, not silent)");
    true
}

/// C.1 — stdout is not a tty: the explanation must reach the *inherited*
/// stderr, and the exit code must say it failed. This is the arc-1b silent
/// exit, as a gate.
#[test]
fn no_tty_fails_loudly_on_stderr() {
    let sandbox = Sandbox::new("notty");
    let mut cmd = Command::new(bin());
    cmd.arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    sandbox.env(&mut cmd, "notty");
    let t = Instant::now();
    let out = cmd.output().expect("run gridwatch");
    assert!(
        t.elapsed() < Duration::from_secs(2),
        "took {:?}",
        t.elapsed()
    );
    assert_eq!(out.status.code(), Some(1), "exit code");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("not a tty"),
        "stderr must explain the failure, got: {err:?}"
    );
    assert!(
        !sandbox.log().exists() || std::fs::read_to_string(sandbox.log()).unwrap().is_empty(),
        "the failure must not be hidden in the log file"
    );
}

/// C.2 — a one-row terminal shows the too-small notice, not a blank screen.
#[test]
fn one_row_terminal_explains_itself() {
    if skip("one_row_terminal_explains_itself") {
        return;
    }
    let s = Session::start("onerow", 1, 120, "run --demo");
    let seen = s.wait_for(Duration::from_secs(3), |t| {
        t.contains("gridwatch needs at least")
    });
    assert!(
        seen.is_some(),
        "no too-small notice; screen: {:?}",
        s.screen()
    );
}

/// C.3 — a normal terminal draws the cpu tile within a second (P18 as a test).
#[test]
fn first_frame_arrives_within_a_second() {
    if skip("first_frame_arrives_within_a_second") {
        return;
    }
    let s = Session::start("first", 24, 80, "run --demo");
    let t = Instant::now();
    let seen = s.wait_for(Duration::from_secs(3), |t| t.contains("CPU"));
    let took = t.elapsed();
    assert!(seen.is_some(), "no cpu tile; screen: {:?}", s.screen());
    assert!(took < Duration::from_secs(1), "first frame took {took:?}");
}

/// C.4 — resizing the pty mid-run changes what is drawn: the tile follows
/// the terminal, and a too-small resize shows the notice.
#[test]
fn resize_mid_run_is_followed() {
    if skip("resize_mid_run_is_followed") {
        return;
    }
    let s = Session::start("resize", 24, 80, "run --demo");
    assert!(
        s.wait_for(Duration::from_secs(3), |t| t.contains("CPU"))
            .is_some()
    );
    let before = s.raw().len();
    s.resize(50, 200);
    let seen = s.wait_for(Duration::from_secs(3), |t| t.contains("CCD0"));
    assert!(seen.is_some(), "no cores tier after growing to 200x50");
    assert!(
        s.raw().len() > before,
        "nothing was redrawn after the resize"
    );
    s.resize(1, 158);
    let seen = s.wait_for(Duration::from_secs(3), |t| {
        t.contains("gridwatch needs at least")
    });
    assert!(
        seen.is_some(),
        "no too-small notice after shrinking to 158x1"
    );
    s.resize(20, 60);
    let mark = s.raw().len();
    let seen = s.wait_for(Duration::from_secs(3), |_| s.raw().len() > mark);
    assert!(seen.is_some(), "no redraw after growing back to 60x20");
}

/// C.5 — `q` quits cleanly: exit 0, the alternate screen is left, mouse
/// capture is disabled, and the log holds no errors.
#[test]
fn quit_restores_the_terminal_and_logs_nothing_bad() {
    if skip("quit_restores_the_terminal_and_logs_nothing_bad") {
        return;
    }
    let mut s = Session::start("quit", 24, 80, "run --demo");
    assert!(
        s.wait_for(Duration::from_secs(3), |t| t.contains("CPU"))
            .is_some()
    );
    s.keys("q");
    let (code, raw, log) = s.finish();
    assert_eq!(code, 0, "exit code after q");
    let raw_s = String::from_utf8_lossy(&raw);
    assert!(raw_s.contains("\x1b[?1049l"), "alternate screen not left");
    assert!(raw_s.contains("\x1b[?1000l"), "mouse capture not disabled");
    let bad: Vec<&str> = log.lines().filter(|l| l.contains("ERROR")).collect();
    assert!(bad.is_empty(), "errors in the log: {bad:?}");
}

/// C.6 (arc 2a, 2b) — `--demo` draws **both** process tables in the two 6x3
/// tiles: htop's header and the synthetic game row, and nvtop's `GPU MEM`
/// column with the game's merged `Both G+C` row.
#[test]
fn demo_draws_the_process_table() {
    if skip("demo_draws_the_process_table") {
        return;
    }
    let s = Session::start("table", 70, 250, "run --demo");
    let seen = s.wait_for(Duration::from_secs(4), |t| {
        t.contains("Command")
            && t.contains("/opt/game/bin/game")
            && t.contains("GPU MEM")
            && t.contains("Both G+C")
    });
    assert!(
        seen.is_some(),
        "no process tables; screen: {:?}",
        s.screen()
    );
}

/// C.10 (arc 3a) — `--demo` draws the pins tile, and at 250×70 the scripted
/// overload raises the red banner after ≈ 22 s; `A` opens the alerts overlay.
#[test]
fn demo_raises_the_pins_banner_and_a_opens_the_overlay() {
    if skip("demo_raises_the_pins_banner_and_a_opens_the_overlay") {
        return;
    }
    let mut s = Session::start("banner", 70, 250, "run --demo");
    let seen = s.wait_for(Duration::from_secs(4), |t| t.contains("balance"));
    assert!(seen.is_some(), "no pins tile; screen: {:?}", s.screen());
    let seen = s.wait_for(Duration::from_secs(30), |t| t.contains("ALERT: OVERLOAD"));
    assert!(
        seen.is_some(),
        "no banner by 30 s; screen: {:?}",
        s.screen()
    );
    // The typescript keeps every frame, so "the banner is gone" cannot be
    // read from it; the shell test covers `a`. Here: `A` opens the overlay.
    s.keys("A");
    let overlay = s.wait_for(Duration::from_secs(3), |t| t.contains("Esc to close"));
    assert!(
        overlay.is_some(),
        "no alerts overlay after `A`; screen: {:?}",
        s.screen()
    );
    s.keys("\x1b");
    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    let bad: Vec<&str> = log.lines().filter(|l| l.contains("ERROR")).collect();
    assert!(bad.is_empty(), "errors in the log: {bad:?}");
}

/// C.11 (arc 3a) — `--replay fixtures/journals/synth-overload.jsonl --speed 10`
/// reaches the banner: the alert lines replay through the control channel.
#[test]
fn replay_of_the_overload_fixture_raises_the_banner() {
    if skip("replay_of_the_overload_fixture_raises_the_banner") {
        return;
    }
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/journals/synth-overload.jsonl");
    let args = format!("run --replay {} --speed 10", fixture.display());
    let mut s = Session::start("overload", 70, 250, &args);
    let seen = s.wait_for(Duration::from_secs(8), |t| t.contains("ALERT: OVERLOAD"));
    assert!(
        seen.is_some(),
        "no banner from the fixture; screen: {:?}",
        s.screen()
    );
    s.keys("q");
    let (code, _, _) = s.finish();
    assert_eq!(code, 0);
}

/// C.7 (arc 2a) — `--replay FILE --speed 0` reaches a frame from the
/// recorded fixture, and the journal source reports the end of the file.
#[test]
fn replay_reaches_a_frame() {
    if skip("replay_reaches_a_frame") {
        return;
    }
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/journals/torch-idle.jsonl");
    let args = format!("run --replay {} --speed 0", fixture.display());
    let mut s = Session::start("replay", 70, 250, &args);
    let seen = s.wait_for(Duration::from_secs(5), |t| t.contains("CCD0"));
    assert!(
        seen.is_some(),
        "no cpu tile from the journal; screen: {:?}",
        s.screen()
    );
    let seen = s.wait_for(Duration::from_secs(5), |t| t.contains("end of journal"));
    assert!(
        seen.is_some(),
        "the sources tile never showed the end of the journal"
    );
    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    let bad: Vec<&str> = log.lines().filter(|l| l.contains("ERROR")).collect();
    assert!(bad.is_empty(), "errors in the log: {bad:?}");
}

/// C.8 (arc 2a) — `--record FILE` writes a journal with a header and lines,
/// and the recording toast reaches the screen.
#[test]
fn record_writes_a_journal() {
    if skip("record_writes_a_journal") {
        return;
    }
    let out = std::env::temp_dir().join(format!("gridwatch-pty-rec-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let args = format!("run --demo --record {}", out.display());
    let mut s = Session::start("record", 40, 131, &args);
    let seen = s.wait_for(Duration::from_secs(4), |t| t.contains("recording"));
    assert!(
        seen.is_some(),
        "no recording toast; screen: {:?}",
        s.screen()
    );
    std::thread::sleep(Duration::from_millis(1800));
    s.keys("q");
    let (code, _, _) = s.finish();
    assert_eq!(code, 0);
    let text = std::fs::read_to_string(&out).expect("the journal file");
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() >= 3, "too few lines: {}", lines.len());
    assert!(lines[0].starts_with(r#"{"v":1,"#), "header: {}", lines[0]);
    assert!(
        lines.iter().any(|l| l.contains("\"b\":{\"src\":\"cpu\"")),
        "no cpu batch"
    );
    assert!(!text.contains("proc.table"), "tables are off by default");
    let _ = std::fs::remove_file(&out);
}

/// C.9 (arc 2a review) — `--replay` on a file that is not a journal is refused
/// on stderr before the alternate screen (the tty check comes first, so this
/// runs under a pty), never replayed into a dashboard of dashes.
#[test]
fn replay_of_a_non_journal_is_refused_loudly() {
    if skip("replay_of_a_non_journal_is_refused_loudly") {
        return;
    }
    let readme = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md");
    let empty = std::env::temp_dir().join(format!("gridwatch-empty-{}.jsonl", std::process::id()));
    std::fs::write(&empty, "").unwrap();
    for (tag, path, expect) in [
        ("notjournal", readme, "not a gridwatch journal"),
        ("emptyjournal", empty.clone(), "empty file"),
    ] {
        let args = format!("run --replay {}", path.display());
        let s = Session::start(tag, 24, 100, &args);
        let seen = s.wait_for(Duration::from_secs(3), |t| t.contains(expect));
        assert!(
            seen.is_some(),
            "no refusal for {}; screen: {:?}",
            path.display(),
            s.screen()
        );
        let (code, raw, _) = s.finish();
        assert_eq!(code, 1, "exit code for {}", path.display());
        assert!(
            !String::from_utf8_lossy(&raw).contains("\x1b[?1049h"),
            "the alternate screen must not be entered"
        );
    }
    let _ = std::fs::remove_file(&empty);
}

/// C.12 (arc 3b) — hot reload under a pty: editing `config.toml` swaps the
/// theme within two seconds and toasts; a broken file keeps the old state and
/// names the line; a `layout.toml` that appears is picked up; `T` reloads
/// the theme on demand.
#[test]
fn editing_the_config_files_reloads_within_two_seconds() {
    if skip("editing_the_config_files_reloads_within_two_seconds") {
        return;
    }
    let mut s = Session::start("reload", 70, 250, "run --demo");
    let dir = s.sandbox.root.join("config/gridwatch");
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("config.toml");
    let seen = s.wait_for(Duration::from_secs(4), |t| {
        t.contains("retrowave · configured")
    });
    assert!(seen.is_some(), "no first frame; screen: {:?}", s.screen());
    // The file appears with another theme: reload + swap.
    let modern = gridwatch_app::config::DEFAULT_CONFIG
        .replace("theme = \"retrowave\"", "theme = \"modern\"");
    std::fs::write(&cfg, &modern).unwrap();
    let seen = s.wait_for(Duration::from_secs(2), |t| {
        t.contains("config.toml reloaded")
    });
    assert!(
        seen.is_some(),
        "no reload toast in 2 s; screen: {:?}",
        s.screen()
    );
    let seen = s.wait_for(Duration::from_secs(2), |t| {
        t.contains("modern · configured")
    });
    assert!(
        seen.is_some(),
        "theme did not follow the file; screen: {:?}",
        s.screen()
    );
    // A broken edit: kept, and the toast says where.
    std::fs::write(&cfg, "schema = 1\ntheme = \"modern\"\nfps = \"thirty\"\n").unwrap();
    let seen = s.wait_for(Duration::from_secs(2), |t| {
        t.contains("kept the old config")
    });
    assert!(seen.is_some(), "no error toast; screen: {:?}", s.screen());
    assert!(
        s.screen().contains("config.toml:3:"),
        "the toast does not name the line; screen: {:?}",
        s.screen()
    );
    // layout.toml appearing is a change too.
    std::fs::write(&cfg, &modern).unwrap();
    std::fs::write(
        dir.join("layout.toml"),
        gridwatch_app::config::DEFAULT_LAYOUT,
    )
    .unwrap();
    let seen = s.wait_for(Duration::from_secs(2), |t| {
        t.contains("layout.toml reloaded")
    });
    assert!(seen.is_some(), "no layout toast; screen: {:?}", s.screen());
    // `T`: reload the theme now.
    s.keys("T");
    let seen = s.wait_for(Duration::from_secs(2), |t| {
        t.contains("theme reloaded: modern")
    });
    assert!(seen.is_some(), "no `T` toast; screen: {:?}", s.screen());
    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    let bad: Vec<&str> = log.lines().filter(|l| l.contains("ERROR")).collect();
    assert!(bad.is_empty(), "errors in the log: {bad:?}");
}

/// C.13 (arc 3b) — `gridwatch doctor --offline` prints every capability with
/// a reason and a fix and exits 0. `--offline` skips the live probes: the
/// exporter GET and `detect_bus`, which opens `/dev/i2c-*` — never from a
/// test on torch (MACHINE.md); the live table is run by hand.
#[test]
fn doctor_prints_every_capability_with_a_fix() {
    let out = Command::new(bin())
        .args(["doctor", "--offline"])
        .env("COLORTERM", "")
        .output()
        .expect("run doctor");
    assert!(out.status.success(), "{:?}", out);
    let text = String::from_utf8_lossy(&out.stdout);
    for cap in [
        "Procfs",
        "Nvml",
        "I2cNvidia",
        "AstralExporter",
        "PwRecord",
        "TrueColor",
    ] {
        assert!(text.contains(cap), "no row for {cap}:\n{text}");
    }
    assert!(text.contains("✗ TrueColor"), "{text}");
    assert!(text.contains("fix: use a truecolor terminal"), "{text}");
    assert!(text.contains("live probes skipped (--offline)"), "{text}");
}

/// C.14 (arc 3b review) — `config check --theme` fails loudly on a theme that
/// does not load (exit 1, the reason on stderr), as `run` would.
#[test]
fn config_check_fails_on_a_theme_that_does_not_load() {
    let sandbox = Sandbox::new("check");
    let mut cmd = Command::new(bin());
    cmd.args(["config", "check", "--theme", "nonexistent"]);
    sandbox.env(&mut cmd, "check");
    let out = cmd.output().expect("run config check");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no built-in theme 'nonexistent'"), "{err}");
    let mut cmd = Command::new(bin());
    cmd.args(["config", "check", "--theme", "phosphor-green"]);
    sandbox.env(&mut cmd, "check");
    let out = cmd.output().expect("run config check");
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("contrast (WCAG 2.1):"), "{text}");
    assert!(text.contains("text on panel: 15.93:1  ok"), "{text}");
}

/// C.15 (arc 4a) — edit mode under a pty: `e`, `L` twice on the cpu tile
/// after narrowing it, `w`, and the sandbox's `layout.toml` carries the
/// move; `q` exits 0 with no ERROR in the log.
#[test]
fn edit_mode_saves_the_moved_tile() {
    if skip("edit_mode_saves_the_moved_tile") {
        return;
    }
    let mut s = Session::start("edit", 70, 250, "run --demo");
    let seen = s.wait_for(Duration::from_secs(4), |t| {
        t.contains("retrowave · configured")
    });
    assert!(seen.is_some(), "no first frame; screen: {:?}", s.screen());
    // The typescript carries only the cells ratatui rewrote (a diff, not a
    // screen), so assertions key on words that are new as a whole.
    s.keys("e");
    let seen = s.wait_for(Duration::from_secs(2), |t| t.contains("EDIT"));
    assert!(seen.is_some(), "no edit key bar; screen: {:?}", s.screen());
    // Narrow twice (Ctrl-h = 0x08), then move right twice; each key redraws.
    s.keys("\x08\x08LL");
    std::thread::sleep(Duration::from_millis(600));
    s.keys("w");
    let seen = s.wait_for(Duration::from_secs(2), |t| t.contains("saved"));
    assert!(seen.is_some(), "no save toast; screen: {:?}", s.screen());
    let layout = s.sandbox.root.join("config/gridwatch/layout.toml");
    let text = std::fs::read_to_string(&layout).expect("layout.toml written");
    assert!(
        text.contains("id = \"cpu\", at = [2, 0], size = [4, 3]"),
        "{text}"
    );
    assert!(
        !s.sandbox.root.join("config/gridwatch/config.toml").exists(),
        "config.toml must never be written"
    );
    // The watcher must not reload the app's own write.
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !s.screen().contains("layout.toml reloaded"),
        "{}",
        s.screen()
    );
    s.keys("\x1bq");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    let bad: Vec<&str> = log.lines().filter(|l| l.contains("ERROR")).collect();
    assert!(bad.is_empty(), "errors in the log: {bad:?}");
}

/// C.16 (arc 4b) — `--demo --theme matrix` at 250×70 draws rain glyphs within
/// two seconds; the pins banner text reaches the terminal near 22 s; `L`
/// then `q` exits 0 with no ERROR in the log.
#[test]
fn matrix_rain_draws_and_the_banner_comes_through() {
    if skip("matrix_rain_draws_and_the_banner_comes_through") {
        return;
    }
    let mut s = Session::start("matrix", 70, 250, "run --demo --theme matrix");
    let seen = s.wait_for(Duration::from_secs(2), |t| {
        t.chars().any(|c| ('\u{FF66}'..='\u{FF9D}').contains(&c))
    });
    assert!(
        seen.is_some(),
        "no rain in 2 s; screen: {:?}",
        s.screen().chars().take(400).collect::<String>()
    );
    let seen = s.wait_for(Duration::from_secs(30), |t| t.contains("ALERT: OVERLOAD"));
    assert!(seen.is_some(), "no banner by 30 s");
    s.keys("L");
    std::thread::sleep(Duration::from_millis(500));
    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    let bad: Vec<&str> = log.lines().filter(|l| l.contains("ERROR")).collect();
    assert!(bad.is_empty(), "errors in the log: {bad:?}");
}

/// C.17 (arc 4b) — `--no-effects --demo` starts, draws the page, logs no
/// effects notice and exits 0.
#[test]
fn no_effects_runs_plain() {
    if skip("no_effects_runs_plain") {
        return;
    }
    let stats =
        std::env::temp_dir().join(format!("gridwatch-noeffects-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&stats);
    let args = format!("run --demo --no-effects --stats-log {}", stats.display());
    let mut s = Session::start("noeffects", 70, 250, &args);
    let seen = s.wait_for(Duration::from_secs(3), |t| t.contains("CCD0"));
    assert!(seen.is_some(), "no first frame; screen: {:?}", s.screen());
    std::thread::sleep(Duration::from_millis(1500));
    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    assert!(!log.contains("effects off"), "{log}");
    assert!(!log.contains("ERROR"), "{log}");
    // No animation-caused frame at all: the first frame was the full page.
    let text = std::fs::read_to_string(&stats).unwrap_or_default();
    let last = text.lines().last().unwrap_or("");
    assert!(last.contains("\"redraw_anim\":0"), "{last}");
    let _ = std::fs::remove_file(&stats);
}

/// C.18 (arc 5a) — `--demo --page 2` draws the audio tile's `Hz` axis and
/// the bars keep moving: the terminal keeps receiving bar glyphs across
/// half a second, and the stats log counts animation-caused frames.
/// `--no-effects` leaves the visualizer animating (it is data, not an effect).
#[test]
fn demo_audio_page_animates_the_spectrum() {
    if skip("demo_audio_page_animates_the_spectrum") {
        return;
    }
    for extra in ["", " --no-effects"] {
        let stats =
            std::env::temp_dir().join(format!("gridwatch-audio-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&stats);
        let args = format!("run --demo --page 2{extra} --stats-log {}", stats.display());
        let mut s = Session::start("audio", 70, 250, &args);
        let seen = s.wait_for(Duration::from_secs(3), |t| t.contains("Hz"));
        assert!(seen.is_some(), "no Hz axis; screen: {:?}", s.screen());
        // The synth is silent for 1.5 s, then the song: wait past that.
        std::thread::sleep(Duration::from_millis(2500));
        let before = s.raw().len();
        std::thread::sleep(Duration::from_millis(500));
        let after = s.raw();
        let fresh = strip_escapes(&String::from_utf8_lossy(&after[before.min(after.len())..]));
        assert!(
            fresh.contains('█') || fresh.contains('▔') || fresh.contains('▄'),
            "no bar glyphs in the last 500 ms ({} new bytes){extra}: {fresh:?}",
            after.len() - before
        );
        std::thread::sleep(Duration::from_millis(1200));
        s.keys("q");
        let (code, _, log) = s.finish();
        assert_eq!(code, 0);
        assert!(!log.contains("ERROR"), "{log}");
        let text = std::fs::read_to_string(&stats).unwrap_or_default();
        let last = text.lines().last().unwrap_or("");
        let anim: u64 = last
            .split("\"redraw_anim\":")
            .nth(1)
            .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        assert!(anim > 20, "animation frames{extra}: {last}");
        let _ = std::fs::remove_file(&stats);
    }
}

/// C.19 (arc 5a) — `--demo --fps 60 --page 2` draws and `q` exits 0.
#[test]
fn demo_audio_at_sixty_fps_exits_cleanly() {
    if skip("demo_audio_at_sixty_fps_exits_cleanly") {
        return;
    }
    let mut s = Session::start("audio60", 70, 250, "run --demo --fps 60 --page 2");
    let seen = s.wait_for(Duration::from_secs(3), |t| t.contains("Hz"));
    assert!(seen.is_some(), "no first frame; screen: {:?}", s.screen());
    std::thread::sleep(Duration::from_millis(3000));
    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    assert!(!log.contains("ERROR"), "{log}");
}

/// C.20 (arc 5b) — `--demo` shows the sensors strip on the Overview (the
/// default layout's `temps` slot), and `doctor --offline` lists the chips
/// the walker found on this machine (a sysfs read — allowed).
#[test]
fn demo_shows_the_sensors_tile_and_doctor_lists_the_chips() {
    if skip("demo_shows_the_sensors_tile_and_doctor_lists_the_chips") {
        return;
    }
    let mut s = Session::start("sensors", 70, 250, "run --demo");
    let seen = s.wait_for(Duration::from_secs(3), |t| {
        t.contains("k10temp") || t.contains("spd5118")
    });
    assert!(seen.is_some(), "no sensors tile; screen: {:?}", s.screen());
    let screen = s.screen();
    assert!(screen.contains("sensors"), "the tile's title: {screen}");
    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    assert!(!log.contains("ERROR"), "{log}");

    // `doctor --offline` still runs the sources' sysfs probes.
    let out = std::process::Command::new(bin())
        .args(["doctor", "--offline"])
        .output()
        .expect("doctor runs");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Hwmon"), "{text}");
    assert!(
        text.contains("chips") || text.contains("no hwmon"),
        "the walker's row: {text}"
    );
}

/// C.21 (arc 6) — `--demo --page 2` shows the winamp tile with the track
/// the synthetic player is on, and the tile animates: the stats log counts
/// animation-caused frames while it plays. `q` exits 0.
#[test]
fn demo_winamp_draws_and_animates() {
    if skip("demo_winamp_draws_and_animates") {
        return;
    }
    let stats = std::env::temp_dir().join(format!("gridwatch-winamp-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&stats);
    let args = format!("run --demo --page 2 --stats-log {}", stats.display());
    let mut s = Session::start("winamp", 70, 250, &args);
    // The theme decides the case of a title, so match either.
    let seen = s.wait_for(Duration::from_secs(4), |t| {
        let t = t.to_lowercase();
        t.contains("crate of many") || t.contains("interlude") || t.contains("groove salad")
    });
    assert!(
        seen.is_some(),
        "no track on the tile; screen: {:?}",
        s.screen()
    );
    let screen = s.screen().to_lowercase();
    assert!(screen.contains("now playing"), "the tile's title");
    assert!(screen.contains("vol"), "the volume row");
    std::thread::sleep(Duration::from_millis(2500));
    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    assert!(!log.contains("ERROR"), "{log}");
    let text = std::fs::read_to_string(&stats).unwrap_or_default();
    let last = text.lines().last().unwrap_or("");
    let anim: u64 = last
        .split("\"redraw_anim\":")
        .nth(1)
        .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    assert!(anim > 10, "the tile animates: {last}");
    let _ = std::fs::remove_file(&stats);
}

/// C.22 (arc 6) — the transport keys reach the tile: capture it, press `c`
/// (pause) and `x` (play). The demo player ignores them (it is a synth), so
/// the test asserts the keys are consumed — the key bar stays the tile's and
/// nothing is logged as an error.
#[test]
fn demo_winamp_transport_keys_are_consumed() {
    if skip("demo_winamp_transport_keys_are_consumed") {
        return;
    }
    let mut s = Session::start("winampkeys", 70, 250, "run --demo --page 2");
    let seen = s.wait_for(Duration::from_secs(4), |t| {
        t.to_lowercase().contains("now playing")
    });
    assert!(seen.is_some(), "no winamp tile; screen: {:?}", s.screen());
    // Focus the tile and capture it: the key bar becomes the component's.
    s.keys("\r");
    let seen = s.wait_for(Duration::from_secs(2), |t| {
        t.to_lowercase().contains("esc release")
    });
    assert!(seen.is_some(), "no capture; screen: {:?}", s.screen());
    let seen = s.wait_for(Duration::from_secs(2), |t| {
        t.to_lowercase().contains("play pause stop")
    });
    assert!(
        seen.is_some(),
        "the transport hints are not in the key bar: {:?}",
        s.screen()
    );
    s.keys("cxb");
    std::thread::sleep(Duration::from_millis(400));
    // Esc alone, then a pause: `Esc` immediately followed by `q` reads as
    // Alt+q to a terminal parser, and nothing would quit.
    s.keys("\x1b");
    std::thread::sleep(Duration::from_millis(300));
    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    assert!(!log.contains("ERROR"), "{log}");
}

/// C.23 (arc 7a) — `--demo` shows the net tile with torch's interfaces and
/// the probe strip; `q` exits 0. Then `doctor --offline` counts the real
/// interfaces (a sysfs read, so it runs offline).
#[test]
fn demo_shows_the_net_tile_and_doctor_counts_interfaces() {
    if skip("demo_shows_the_net_tile_and_doctor_counts_interfaces") {
        return;
    }
    let mut s = Session::start("net", 70, 250, "run --demo");
    let seen = s.wait_for(Duration::from_secs(3), |t| {
        let t = t.to_lowercase();
        t.contains("eno1") && t.contains("gateway")
    });
    assert!(seen.is_some(), "no net tile; screen: {:?}", s.screen());
    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    assert!(!log.contains("ERROR"), "{log}");

    let out = std::process::Command::new(bin())
        .args(["doctor", "--offline"])
        .output()
        .expect("doctor runs");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("interfaces"), "the net source's row: {text}");
}

/// C.24 (arc 7a) — captured, `a` lists the hidden interfaces and `s`
/// changes the order.
#[test]
fn demo_net_keys_show_all_and_sort() {
    if skip("demo_net_keys_show_all_and_sort") {
        return;
    }
    let mut s = Session::start("netkeys", 70, 250, "run --demo");
    let seen = s.wait_for(Duration::from_secs(3), |t| {
        t.to_lowercase().contains("eno1")
    });
    assert!(seen.is_some(), "no net tile; screen: {:?}", s.screen());
    // A layout with the net tile alone, so `Enter` captures it whatever
    // the shipped page's order is (the reload path is C.12's).
    let dir = s.sandbox.root.join("config/gridwatch");
    std::fs::create_dir_all(&dir).unwrap();
    // Keep the shipped grid block (a layout without it does not load) and
    // replace the pages with one full-screen net tile.
    let shipped = gridwatch_app::config::DEFAULT_LAYOUT;
    let grid = &shipped[..shipped.find("[[pages]]").expect("a pages section")];
    std::fs::write(
        dir.join("layout.toml"),
        format!(
            "{grid}[[pages]]\nname = \"net\"\nplace = [{{ id = \"lan\", at = [0, 0], size = [12, 6] }}]\n"
        ),
    )
    .unwrap();
    let seen = s.wait_for(Duration::from_secs(4), |t| {
        t.contains("layout.toml reloaded")
    });
    assert!(seen.is_some(), "no reload toast; screen: {:?}", s.screen());
    s.keys("\r");
    std::thread::sleep(Duration::from_millis(400));
    s.keys("a");
    let seen = s.wait_for(Duration::from_secs(2), |t| t.contains("br-"));
    assert!(
        seen.is_some(),
        "`a` did not reveal the hidden bridge: {:?}",
        s.screen()
    );
    // `s` reorders the table. The typescript is a diff stream, so the
    // footer's "sort traffic" → "sort name" arrives as the bare word, which
    // no assertion could tell from any other "name" on screen; the ordering
    // is pinned by the unit test `the_sort_puts_the_busy_link_first`. Here
    // the key is pressed only to prove the run survives it.
    s.keys("s");
    std::thread::sleep(Duration::from_millis(400));
    s.keys("\x1b");
    std::thread::sleep(Duration::from_millis(300));
    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    assert!(!log.contains("ERROR"), "{log}");
}

/// C.25 (arc 7b) — a `[[rules]]` entry written into the sandbox reloads and
/// fires against the demo data: the banner appears, `a` acknowledges it,
/// and `config check` lists the rule in the words the engine uses.
#[test]
fn a_config_rule_fires_against_the_demo_data() {
    if skip("a_config_rule_fires_against_the_demo_data") {
        return;
    }
    let mut s = Session::start("rules", 70, 250, "run --demo");
    let seen = s.wait_for(Duration::from_secs(4), |t| t.contains("retrowave"));
    assert!(seen.is_some(), "no first frame; screen: {:?}", s.screen());
    let dir = s.sandbox.root.join("config/gridwatch");
    std::fs::create_dir_all(&dir).unwrap();
    // The demo GPU runs hot enough that "over 20°C" is certain, and a
    // one-second hold keeps the test short.
    let cfg = format!(
        "{}\n[[rules]]\nname = \"demo gpu rule\"\nkey = \"gpu.temp_c\"\nop = \">\"\nvalue = 20\nfor_s = 1\nseverity = \"warn\"\nmessage = \"the demo gpu is at {{value}}\"\n",
        gridwatch_app::config::DEFAULT_CONFIG
    );
    std::fs::write(dir.join("config.toml"), cfg).unwrap();
    let seen = s.wait_for(Duration::from_secs(6), |t| t.contains("demo gpu rule"));
    assert!(
        seen.is_some(),
        "the rule did not raise a banner; screen: {:?}",
        s.screen()
    );
    // `a` acknowledges it, as it would any alert.
    s.keys("a");
    std::thread::sleep(Duration::from_millis(500));

    // `config check` says what will fire, not just how many. It runs while
    // the session is alive: the sandbox deletes itself on `finish`.
    let out = std::process::Command::new(bin())
        .args(["config", "check"])
        .env("XDG_CONFIG_HOME", s.sandbox.root.join("config"))
        .env("XDG_STATE_HOME", s.sandbox.root.join("state"))
        .output()
        .expect("config check runs");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("rules: 1"), "{text}");
    assert!(text.contains("gpu.temp_c > 20"), "{text}");
    assert!(text.contains("Warn"), "{text}");

    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    assert!(!log.contains("ERROR"), "{log}");
}

/// C.26 (arc 8a) — zoom the htop tile, search, and open the signal menu
/// **and cancel it**. No signal is ever sent: the run is fenced to a pid
/// nothing here spawned (`GRIDWATCH_ACTION_ALLOW_PIDS`), so even a
/// confirmed action would be refused by the executor.
#[test]
fn zoomed_htop_searches_and_cancels_the_signal_menu() {
    if skip("zoomed_htop_searches_and_cancels_the_signal_menu") {
        return;
    }
    let mut s = Session::start_with_env(
        "htopfull",
        70,
        250,
        "run --demo",
        &[("GRIDWATCH_ACTION_ALLOW_PIDS", "424242")],
    );
    let seen = s.wait_for(Duration::from_secs(4), |t| t.contains("retrowave"));
    assert!(seen.is_some(), "no first frame; screen: {:?}", s.screen());
    // The htop tile is the first placement on the Overview. Zoom first,
    // then capture: a captured tile sees every key, so `z` would go to the
    // component and be ignored.
    s.keys("z");
    std::thread::sleep(Duration::from_millis(300));
    s.keys("\r");
    let seen = s.wait_for(Duration::from_secs(3), |t| {
        t.contains("F9") && t.to_lowercase().contains("kill")
    });
    assert!(
        seen.is_some(),
        "no F-key bar in the zoomed tile: {:?}",
        s.screen()
    );
    // The demo's process table arrives on the scan cadence; until it does
    // the tile says so, and a picker has nothing to act on.
    let seen = s.wait_for(Duration::from_secs(6), |t| t.contains("TIME+"));
    assert!(
        seen.is_some(),
        "no process table in the zoomed tile: {:?}",
        s.screen()
    );
    // Search finds the game the demo set carries.
    s.keys("/game");
    let seen = s.wait_for(Duration::from_secs(2), |t| t.contains("search: game"));
    assert!(seen.is_some(), "no search line: {:?}", s.screen());
    s.keys("\r");
    std::thread::sleep(Duration::from_millis(200));
    // A picker opens and Esc closes it. Nothing is ever confirmed.
    // `a` (the affinity picker) rather than F9 (the signal one): both go
    // through the same open/answer/cancel path, and a plain character
    // proves the key reached the tile without depending on how this
    // terminal encodes a function key.
    s.keys("a");
    // The typescript is a diff stream, so the title line can arrive in
    // pieces; the list of CPUs is what lands whole.
    let seen = s.wait_for(Duration::from_secs(2), |t| {
        t.contains("Space choose") && t.contains("Esc cancel")
    });
    assert!(seen.is_some(), "no picker: {:?}", s.screen());
    s.keys("\x1b");
    std::thread::sleep(Duration::from_millis(300));
    s.keys("\x1b");
    std::thread::sleep(Duration::from_millis(200));
    s.keys("z");
    std::thread::sleep(Duration::from_millis(200));
    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    assert!(!log.contains("ERROR"), "{log}");
}

/// C.27 (arc 8a) — `--readonly` refuses an action and says what it would
/// have done. F8 is the one action that does not ask, so it is the one
/// that proves the refusal happens before the question.
#[test]
fn readonly_refuses_an_action_and_says_so() {
    if skip("readonly_refuses_an_action_and_says_so") {
        return;
    }
    let mut s = Session::start_with_env(
        "readonly",
        70,
        250,
        "run --demo --readonly",
        &[("GRIDWATCH_ACTION_ALLOW_PIDS", "424242")],
    );
    let seen = s.wait_for(Duration::from_secs(4), |t| t.contains("retrowave"));
    assert!(seen.is_some(), "no first frame; screen: {:?}", s.screen());
    s.keys("z");
    std::thread::sleep(Duration::from_millis(300));
    s.keys("\r");
    let seen = s.wait_for(Duration::from_secs(3), |t| t.contains("F9"));
    assert!(seen.is_some(), "not zoomed: {:?}", s.screen());
    let seen = s.wait_for(Duration::from_secs(6), |t| t.contains("TIME+"));
    assert!(seen.is_some(), "no process table: {:?}", s.screen());
    // Put the cursor on a row, then ask for something that changes it.
    // `i` (the I/O priority picker) confirms; under `--readonly` the
    // refusal happens before any question, which is what this checks.
    s.keys("\x1b[B"); // Down
    std::thread::sleep(Duration::from_millis(200));
    s.keys("i");
    let seen = s.wait_for(Duration::from_secs(2), |t| {
        t.contains("I/O priority for") || t.contains("best-effort")
    });
    assert!(seen.is_some(), "no io picker: {:?}", s.screen());
    s.keys("\r");
    let seen = s.wait_for(Duration::from_secs(2), |t| t.contains("read-only"));
    assert!(seen.is_some(), "no read-only refusal: {:?}", s.screen());
    s.keys("\x1b");
    std::thread::sleep(Duration::from_millis(200));
    s.keys("z");
    std::thread::sleep(Duration::from_millis(200));
    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    assert!(!log.contains("ERROR"), "{log}");
}

/// C.28 (arc 8b) — the example plugin, running for real inside the app: the
/// tile draws the tree it sends, `config check` lists the manifest it would
/// accept, and the metric it publishes reaches the sources tile.
#[test]
fn the_example_plugin_draws_in_a_real_terminal() {
    if skip("the_example_plugin_draws_in_a_real_terminal") {
        return;
    }
    if std::process::Command::new("python3")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| !s.success())
        .unwrap_or(true)
    {
        eprintln!("skipping: no python3");
        return;
    }
    let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/examples/weather.py")
        .canonicalize()
        .expect("the example plugin is in the repository");
    let sandbox = Sandbox::new("plugin");
    let dir = sandbox.root.join("config/gridwatch");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.toml"),
        format!(
            "schema = 1\ntheme = \"mono\"\n\n[[plugins]]\nid = \"weather\"\nargv = [\"python3\", {:?}]\n\n[[components]]\nid = \"outside\"\nkind = \"weather.weather\"\n",
            example.to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("layout.toml"),
        "schema = 1\n\n[grid]\ncolumns = 12\nrows = 6\n\n[[pages]]\nname = \"Plugin\"\nplace = [\n  { id = \"outside\", at = [0, 0], size = [4, 2] },\n  { kind = \"sources\", at = [4, 0], size = [4, 2] },\n]\n",
    )
    .unwrap();
    // `config check` first, on the same config the session will run: it must
    // start the plugin and accept the manifest.
    let out = std::process::Command::new(bin())
        .args(["config", "check"])
        .env("XDG_CONFIG_HOME", sandbox.root.join("config"))
        .env("XDG_STATE_HOME", sandbox.root.join("state"))
        .output()
        .expect("config check runs");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("plugins: 1"), "{text}");
    assert!(text.contains("kind `weather.weather`"), "{text}");

    let mut s = Session::start_in("plugin-run", sandbox, 40, 160, "run");
    // The tile draws the plugin's own tree, and the temperature is in it.
    let seen = s.wait_for(Duration::from_secs(8), |t| t.contains("°C"));
    assert!(
        seen.is_some(),
        "the plugin tile drew nothing; screen: {:?}",
        s.screen()
    );
    // And the metric it published reached the store, so the sources tile
    // lists it beside the built-ins.
    let seen = s.wait_for(Duration::from_secs(6), |t| t.contains("weather"));
    assert!(
        seen.is_some(),
        "the plugin's source is not listed; screen: {:?}",
        s.screen()
    );
    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    assert!(!log.contains("ERROR"), "{log}");
}

/// C.29 (arc 8b) — `--replay` starts no plugin. The journal is the only
/// source of a replayed frame (D47's byte-identical promise, D53's reason
/// for leaving the watcher out), so a child process must not run: this
/// plugin's first act is to leave a file behind, and the file must not be
/// there.
#[test]
fn replay_starts_no_plugin() {
    if skip("replay_starts_no_plugin") {
        return;
    }
    let sandbox = Sandbox::new("replay-plugin");
    let dir = sandbox.root.join("config/gridwatch");
    std::fs::create_dir_all(&dir).unwrap();
    let marker = sandbox.root.join("the-plugin-ran");
    // `touch` is not a plugin and will never send a manifest — which is the
    // point: all this fixture does is prove it was executed.
    std::fs::write(
        dir.join("config.toml"),
        format!(
            "schema = 1\ntheme = \"mono\"\n\n[[plugins]]\nid = \"marker\"\nargv = [\"touch\", {:?}]\nhello_ms = 200\n",
            marker.to_string_lossy()
        ),
    )
    .unwrap();
    let journal = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/journals/torch-idle.jsonl");
    let args = format!("run --replay {} --speed 0", journal.display());
    let mut s = Session::start_in("replay-plugin-run", sandbox, 40, 160, &args);
    let seen = s.wait_for(Duration::from_secs(8), |t| t.contains("gridwatch"));
    assert!(seen.is_some(), "no frame; screen: {:?}", s.screen());
    assert!(
        !marker.exists(),
        "a plugin was started under --replay: {}",
        marker.display()
    );
    s.keys("q");
    let (code, _, log) = s.finish();
    assert_eq!(code, 0);
    assert!(!log.contains("ERROR"), "{log}");
}
