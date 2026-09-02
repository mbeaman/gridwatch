//! The `pw-record` child (brief arc 5 seam 2, MACHINE.md's verified line):
//! the command line, the socket check, and the io-thread pump that turns the
//! child's f32 stdout into frames in the SPSC ring. Generic over `Read` so
//! the pump is tested on a generated stream without PipeWire.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

pub const RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
/// Ring capacity in frames per channel (≈ 170 ms at 48 kHz).
pub const RING_FRAMES: usize = 8192;
/// The `--latency` used under `low_latency` (never below the running quantum
/// by default — the option is explicit).
pub const LOW_LATENCY: u32 = 256;

/// `[sources.audio] sink` resolved: `pw-record --target` accepts `auto`, a
/// `node.name` or an `object.serial` (never a node id — the digest).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Auto,
    Name(String),
    Serial(u32),
}

impl Target {
    pub fn parse(s: &str) -> Target {
        let s = s.trim();
        if s.is_empty() || s == "auto" {
            Target::Auto
        } else if let Ok(n) = s.parse::<u32>() {
            Target::Serial(n)
        } else {
            Target::Name(s.to_string())
        }
    }

    pub fn arg(&self) -> String {
        match self {
            Target::Auto => "auto".into(),
            Target::Name(n) => n.clone(),
            Target::Serial(n) => n.to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureArgs {
    pub target: Target,
    pub latency: u32,
    pub low_latency: bool,
}

/// The program and its arguments. `low_latency` wraps in `stdbuf -o0` at
/// `--latency 256`; an explicit sink adds `node.dont-fallback` so an
/// unresolvable target fails loudly instead of silently capturing the
/// default (the digest's gotcha).
pub fn command_line(a: &CaptureArgs) -> (String, Vec<String>) {
    let mut props = String::from(
        "{ stream.capture.sink = true, node.passive = true, node.name = \"gridwatch audio\", application.name = \"gridwatch\"",
    );
    if a.target != Target::Auto {
        props.push_str(", node.dont-fallback = true");
    }
    props.push_str(" }");
    let latency = if a.low_latency {
        LOW_LATENCY
    } else {
        a.latency.clamp(256, 4096)
    };
    let mut argv = vec![
        "--format".to_string(),
        "f32".into(),
        "--rate".into(),
        RATE.to_string(),
        "--channels".into(),
        CHANNELS.to_string(),
        "--raw".into(),
        "--latency".into(),
        latency.to_string(),
        "--target".into(),
        a.target.arg(),
        "-P".into(),
        props,
        "-".into(),
    ];
    if a.low_latency {
        argv.insert(0, "pw-record".into());
        argv.insert(0, "-o0".into());
        ("stdbuf".into(), argv)
    } else {
        ("pw-record".into(), argv)
    }
}

/// `$PIPEWIRE_RUNTIME_DIR` / `$XDG_RUNTIME_DIR` `/pipewire-0`.
pub fn socket_path() -> Option<PathBuf> {
    let dir =
        std::env::var_os("PIPEWIRE_RUNTIME_DIR").or_else(|| std::env::var_os("XDG_RUNTIME_DIR"))?;
    let name = std::env::var("PIPEWIRE_REMOTE").unwrap_or_else(|_| "pipewire-0".into());
    Some(PathBuf::from(dir).join(name))
}

pub fn socket_present() -> bool {
    socket_path().is_some_and(|p| p.exists())
}

pub fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

/// Spawn the child with piped stdout/stderr.
pub fn spawn(a: &CaptureArgs) -> std::io::Result<Child> {
    let (prog, argv) = command_line(a);
    Command::new(prog)
        .args(argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

/// What the io thread and the source thread share: the frame clock and the
/// liveness flag.
#[derive(Debug)]
pub struct Pulse {
    epoch: Instant,
    /// Milliseconds after `epoch` of the last frame; `u64::MAX` = none yet.
    last_ms: AtomicU64,
    pub alive: AtomicBool,
    pub frames: AtomicU64,
}

impl Pulse {
    pub fn new(epoch: Instant) -> Arc<Pulse> {
        Arc::new(Pulse {
            epoch,
            last_ms: AtomicU64::new(u64::MAX),
            alive: AtomicBool::new(true),
            frames: AtomicU64::new(0),
        })
    }

    pub fn mark(&self, now: Instant) {
        let ms = now.saturating_duration_since(self.epoch).as_millis() as u64;
        self.last_ms.store(ms, Ordering::Release);
    }

    /// Age of the last frame at `now`; `None` before the first.
    pub fn age(&self, now: Instant) -> Option<std::time::Duration> {
        let ms = self.last_ms.load(Ordering::Acquire);
        (ms != u64::MAX).then(|| {
            let at = self.epoch + std::time::Duration::from_millis(ms);
            now.saturating_duration_since(at)
        })
    }
}

/// The io thread's body: read interleaved f32le frames from `r` into the
/// ring until EOF or an error. A full ring drops the newest samples (the
/// source thread is late; it will catch up on the next tick) so the child
/// is never back-pressured into altering the graph.
pub fn pump<R: Read>(
    mut r: R,
    ring: &mut rtrb::Producer<f32>,
    pulse: &Pulse,
) -> std::io::Result<()> {
    let mut buf = vec![0u8; 4096 * CHANNELS * 4];
    let mut carry: Vec<u8> = Vec::with_capacity(8);
    let mut total = 0u64;
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) => {
                pulse.alive.store(false, Ordering::Release);
                return Ok(());
            }
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                pulse.alive.store(false, Ordering::Release);
                return Err(e);
            }
        };
        let mut bytes: &[u8] = &buf[..n];
        let mut head = Vec::new();
        if !carry.is_empty() {
            head.extend_from_slice(&carry);
            head.extend_from_slice(bytes);
            carry.clear();
            bytes = &head;
        }
        let whole = bytes.len() / 4 * 4;
        let before = total;
        for c in bytes[..whole].chunks_exact(4) {
            let v = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            if ring.push(v).is_ok() {
                total += 1;
            }
        }
        carry.extend_from_slice(&bytes[whole..]);
        if total > before {
            pulse
                .frames
                .store(total / CHANNELS as u64, Ordering::Relaxed);
            pulse.mark(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_matches_the_verified_capture() {
        let (p, a) = command_line(&CaptureArgs {
            target: Target::Auto,
            latency: 1024,
            low_latency: false,
        });
        assert_eq!(p, "pw-record");
        let s = a.join(" ");
        assert!(s.starts_with("--format f32 --rate 48000 --channels 2 --raw --latency 1024 --target auto -P { stream.capture.sink = true, node.passive = true"));
        assert!(!s.contains("dont-fallback"));
        assert!(s.ends_with(" } -"));
        let (p, a) = command_line(&CaptureArgs {
            target: Target::parse("61"),
            latency: 9000,
            low_latency: true,
        });
        assert_eq!(p, "stdbuf");
        assert_eq!(&a[..2], ["-o0", "pw-record"]);
        let s = a.join(" ");
        assert!(s.contains("--latency 256 --target 61"));
        assert!(s.contains("node.dont-fallback = true"));
        assert_eq!(Target::parse(" auto "), Target::Auto);
        assert_eq!(Target::parse(""), Target::Auto);
        assert_eq!(
            Target::parse("alsa_output.x"),
            Target::Name("alsa_output.x".into())
        );
        let (_, a) = command_line(&CaptureArgs {
            target: Target::Auto,
            latency: 10,
            low_latency: false,
        });
        assert!(a.join(" ").contains("--latency 256 "), "clamped up");
    }

    #[test]
    fn the_pump_turns_bytes_into_frames_and_ends_at_eof() {
        let (mut prod, mut cons) = rtrb::RingBuffer::<f32>::new(64);
        let pulse = Pulse::new(Instant::now());
        // 10 stereo frames, delivered as two odd-sized reads (a carry).
        let samples: Vec<f32> = (0..20).map(|i| i as f32 / 20.0).collect();
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        struct Two<'a>(&'a [u8], usize);
        impl Read for Two<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let n = self.1.min(self.0.len()).min(out.len());
                out[..n].copy_from_slice(&self.0[..n]);
                self.0 = &self.0[n..];
                self.1 = usize::MAX;
                Ok(n)
            }
        }
        assert!(pulse.age(Instant::now()).is_none());
        pump(Two(&bytes, 7), &mut prod, &pulse).unwrap();
        assert!(!pulse.alive.load(Ordering::Acquire));
        assert_eq!(pulse.frames.load(Ordering::Relaxed), 10);
        assert!(pulse.age(Instant::now()).is_some());
        let mut got = Vec::new();
        while let Ok(v) = cons.pop() {
            got.push(v);
        }
        assert_eq!(got, samples);
    }

    #[test]
    fn a_full_ring_drops_the_newest_and_keeps_going() {
        let (mut prod, mut cons) = rtrb::RingBuffer::<f32>::new(4);
        let pulse = Pulse::new(Instant::now());
        let bytes: Vec<u8> = (0..8).flat_map(|i| (i as f32).to_le_bytes()).collect();
        pump(&bytes[..], &mut prod, &pulse).unwrap();
        let mut got = Vec::new();
        while let Ok(v) = cons.pop() {
            got.push(v);
        }
        assert_eq!(got, [0.0, 1.0, 2.0, 3.0]);
    }
}
