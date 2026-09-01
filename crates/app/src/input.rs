//! The input thread (§5, §11): the sole `event::read()` caller; converts
//! crossterm events into the store's serde mirror exactly once.

use crossterm::event::{
    Event as CtEvent, KeyCode as CtKey, KeyEventKind, KeyModifiers, MouseButton as CtButton,
    MouseEventKind as CtMouseKind,
};
use gridwatch_store::{
    Channels, InputEvent, KeyCode, KeyEvent, Mods, MouseButton, MouseEvent, MouseKind,
};

fn mods(m: KeyModifiers) -> Mods {
    Mods {
        ctrl: m.contains(KeyModifiers::CONTROL),
        alt: m.contains(KeyModifiers::ALT),
        shift: m.contains(KeyModifiers::SHIFT),
    }
}

pub fn convert(ev: CtEvent) -> Option<InputEvent> {
    Some(match ev {
        CtEvent::Key(k) => {
            if k.kind == KeyEventKind::Release {
                return None;
            }
            let code = match k.code {
                CtKey::Char(c) => KeyCode::Char(c),
                CtKey::F(n) => KeyCode::F(n),
                CtKey::Enter => KeyCode::Enter,
                CtKey::Esc => KeyCode::Esc,
                CtKey::Tab => KeyCode::Tab,
                CtKey::BackTab => KeyCode::BackTab,
                CtKey::Backspace => KeyCode::Backspace,
                CtKey::Delete => KeyCode::Delete,
                CtKey::Insert => KeyCode::Insert,
                CtKey::Up => KeyCode::Up,
                CtKey::Down => KeyCode::Down,
                CtKey::Left => KeyCode::Left,
                CtKey::Right => KeyCode::Right,
                CtKey::Home => KeyCode::Home,
                CtKey::End => KeyCode::End,
                CtKey::PageUp => KeyCode::PageUp,
                CtKey::PageDown => KeyCode::PageDown,
                _ => return None,
            };
            InputEvent::Key(KeyEvent {
                code,
                mods: mods(k.modifiers),
            })
        }
        CtEvent::Mouse(m) => {
            let btn = |b| match b {
                CtButton::Left => MouseButton::Left,
                CtButton::Right => MouseButton::Right,
                CtButton::Middle => MouseButton::Middle,
            };
            let kind = match m.kind {
                CtMouseKind::Down(b) => MouseKind::Down(btn(b)),
                CtMouseKind::Up(b) => MouseKind::Up(btn(b)),
                CtMouseKind::Drag(b) => MouseKind::Drag(btn(b)),
                CtMouseKind::Moved => MouseKind::Moved,
                CtMouseKind::ScrollUp => MouseKind::ScrollUp,
                CtMouseKind::ScrollDown => MouseKind::ScrollDown,
                _ => return None,
            };
            InputEvent::Mouse(MouseEvent {
                kind,
                x: m.column,
                y: m.row,
                mods: mods(m.modifiers),
            })
        }
        CtEvent::Resize(w, h) => InputEvent::Resize(w, h),
        CtEvent::Paste(s) => InputEvent::Paste(s),
        CtEvent::FocusGained => InputEvent::FocusGained,
        CtEvent::FocusLost => InputEvent::FocusLost,
    })
}

/// Spawn the input thread; exits when the receiver hangs up.
pub fn spawn(ch: Channels) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("gw-input".into())
        .spawn(move || {
            loop {
                match crossterm::event::read() {
                    Ok(ev) => {
                        if let Some(converted) = convert(ev)
                            && ch.input.send(converted).is_err()
                        {
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
        })
        .expect("spawn input thread")
}
