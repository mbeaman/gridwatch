//! The tile a plugin draws through (§4.7, arc 8b).
//!
//! It is an ordinary `Component` and does ordinary component things: it
//! describes, it never does. What it describes is the last tree its plugin
//! sent — held as JSON rather than as a `View`, because `View` carries a
//! `Box<dyn Paint>` and is therefore not `Send`, and nothing about a plugin's
//! tree needs it to be. `tick` is the only `&mut self` the contract offers, so
//! that is where the channel from the host thread is drained.

use std::borrow::Cow;
use std::sync::mpsc::{Receiver, Sender};

use gridwatch_store::{KeyCode, KeyEvent, Mods};
use gridwatch_ui::component::{InputCx, Outcome, Redraw, RenderCx, TickCx, Tier};
use gridwatch_ui::theme::Role;
use gridwatch_ui::view::{Line, Span, View};
use gridwatch_ui::{Component, Manifest};

use super::host::Wish;
use super::proto;

/// What the host thread tells one tile.
#[derive(Clone, Debug)]
pub enum Tell {
    Tree(serde_json::Value),
    Status {
        state: proto::State,
        reason: Option<String>,
        hint: Option<String>,
    },
}

pub struct PluginTile {
    plugin: usize,
    manifest: &'static Manifest,
    tiers: &'static [Tier],
    /// The instance name the host and the plugin both know this tile by — the
    /// same string the placement uses, so a plugin's log line names the tile
    /// you placed.
    instance: String,
    wishes: Sender<Wish>,
    from_host: Receiver<Tell>,
    /// The last tree, as JSON. Turned into a `View` in `view()`; the tree is
    /// capped in depth and item count by the reader, so the conversion is
    /// bounded work on a bounded input.
    tree: Option<serde_json::Value>,
    state: proto::State,
    reason: Option<String>,
    hint: Option<String>,
}

impl PluginTile {
    pub fn new(
        plugin: usize,
        manifest: &'static Manifest,
        tiers: &'static [Tier],
        instance: String,
        wishes: Sender<Wish>,
        from_host: Receiver<Tell>,
    ) -> PluginTile {
        PluginTile {
            plugin,
            manifest,
            tiers,
            instance,
            wishes,
            from_host,
            tree: None,
            state: proto::State::Starting,
            reason: None,
            hint: None,
        }
    }

    /// A one-line say-why view, in the same shape the placeholder chip uses:
    /// the reason, then the fix.
    fn saying(&self, what: &str, role: Role) -> View {
        let mut lines: Vec<Line> = vec![vec![Span::new(role, what.to_string())]];
        if let Some(hint) = &self.hint {
            lines.push(vec![Span::new(Role::TextMuted, hint.clone())]);
        }
        View::Text(lines)
    }
}

impl Component for PluginTile {
    fn manifest(&self) -> &'static Manifest {
        self.manifest
    }

    fn title(&self, _max_width: u16, _cx: &TickCx<'_>) -> Cow<'static, str> {
        Cow::Borrowed(self.manifest.name)
    }

    fn tiers(&self) -> &'static [Tier] {
        self.tiers
    }

    fn tick(&mut self, _cx: &TickCx<'_>) -> Redraw {
        let mut redraw = Redraw::No;
        for tell in self.from_host.try_iter() {
            redraw = Redraw::Yes;
            match tell {
                Tell::Tree(tree) => {
                    self.tree = Some(tree);
                    // A tree is a plugin saying it is working, whatever it
                    // last claimed about itself.
                    if self.state == proto::State::Starting {
                        self.state = proto::State::Ok;
                    }
                }
                Tell::Status {
                    state,
                    reason,
                    hint,
                } => {
                    self.state = state;
                    self.reason = reason;
                    self.hint = hint;
                }
            }
        }
        redraw
    }

    fn view(&self, _cx: &RenderCx<'_>) -> View {
        // A plugin that has said it cannot work says why, in its own words:
        // that is the point of `status`, and it is not a strike (§4.7).
        if matches!(
            self.state,
            proto::State::Unavailable | proto::State::Stopped
        ) {
            let what = self
                .reason
                .clone()
                .unwrap_or_else(|| "the plugin stopped".into());
            return self.saying(&what, Role::Warn);
        }
        let Some(tree) = &self.tree else {
            return self.saying(
                &format!("waiting for {}", self.manifest.name),
                Role::TextMuted,
            );
        };
        match super::tree::read(tree) {
            Ok(view) => view,
            // A tree that will not read is the plugin author's bug, and the
            // sentence that says which shape was wrong is worth more on the
            // tile than in a log file nobody has open.
            Err(why) => View::Text(vec![
                vec![Span::new(Role::Warn, "this view will not read")],
                vec![Span::new(Role::TextMuted, why)],
            ]),
        }
    }

    fn on_key(&mut self, key: KeyEvent, _cx: &InputCx<'_>) -> Outcome {
        let Some(name) = key_name(&key) else {
            return Outcome::Ignored;
        };
        // A plugin only hears keys while its tile is captured, which is what
        // the shell guarantees before it forwards one at all.
        let _ = self.wishes.send(Wish::Key {
            plugin: self.plugin,
            instance: self.instance.clone(),
            key: name,
            mods: mod_names(key.mods),
        });
        Outcome::Consumed
    }
}

/// The key as the wire spells it: a printable character as itself, a named key
/// by its lower-case name.
fn key_name(k: &KeyEvent) -> Option<String> {
    use KeyCode as K;
    Some(match k.code {
        K::Char(c) => c.to_string(),
        K::Enter => "enter".into(),
        K::Tab => "tab".into(),
        K::BackTab => "backtab".into(),
        K::Backspace => "backspace".into(),
        K::Delete => "delete".into(),
        K::Insert => "insert".into(),
        K::Home => "home".into(),
        K::End => "end".into(),
        K::PageUp => "pageup".into(),
        K::PageDown => "pagedown".into(),
        K::Up => "up".into(),
        K::Down => "down".into(),
        K::Left => "left".into(),
        K::Right => "right".into(),
        K::F(n) => format!("f{n}"),
        // `Esc` releases the capture in the shell and never reaches here.
        K::Esc => return None,
    })
}

fn mod_names(m: Mods) -> Vec<&'static str> {
    let mut out = Vec::new();
    if m.ctrl {
        out.push("ctrl");
    }
    if m.alt {
        out.push("alt");
    }
    if m.shift {
        out.push("shift");
    }
    out
}
