//! Terminal lifecycle and panic policy (§11): the app owns raw mode, the
//! alternate screen, mouse capture and focus reporting. Never `ratatui::init`.

use std::io::{Stdout, Write, stdout};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crossterm::event::{
    DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

/// Counts bytes written to the terminal (P6/P7; read by the F12 HUD).
pub struct CountingWriter<W: Write> {
    inner: W,
    pub count: Arc<AtomicU64>,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.count.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

std::thread_local! {
    /// True while a panic should be contained (source threads set this in the
    /// supervisor; the render thread sets it around per-component calls).
    pub static CONTAINED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn restore_terminal(mouse: bool) {
    let mut out = stdout();
    let _ = disable_raw_mode();
    if mouse {
        let _ = execute!(out, DisableMouseCapture);
    }
    let _ = execute!(out, DisableFocusChange, LeaveAlternateScreen);
}

/// One hook for the whole process (§11): contained contexts log and unwind;
/// anything else restores the terminal first, then defers to the default hook.
pub fn install_panic_hook(mouse: bool) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let contained = CONTAINED.try_with(|c| c.get()).unwrap_or(false);
        if contained {
            tracing::error!("contained panic: {info}");
            return;
        }
        restore_terminal(mouse);
        default(info);
    }));
}

pub struct TerminalGuard {
    pub mouse: bool,
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal(self.mouse);
    }
}

pub type AppTerminal = Terminal<CrosstermBackend<CountingWriter<Stdout>>>;

/// Enter raw mode + alternate screen (+ mouse, focus reporting) and build the
/// terminal over a byte-counting writer. The guard restores on drop.
pub fn enter(mouse: bool) -> std::io::Result<(AppTerminal, TerminalGuard, Arc<AtomicU64>)> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableFocusChange)?;
    if mouse {
        execute!(out, EnableMouseCapture)?;
    }
    let count = Arc::new(AtomicU64::new(0));
    let writer = CountingWriter {
        inner: stdout(),
        count: count.clone(),
    };
    let terminal = Terminal::new(CrosstermBackend::new(writer))?;
    Ok((terminal, TerminalGuard { mouse }, count))
}
