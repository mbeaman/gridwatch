//! Edit mode's state (§10, brief arc 4 seam 1, 2, 4): undo/redo over page
//! snapshots, the red ghost after a refused op, the picker, the mouse drag.
//! The page ops themselves are `gridwatch_ui::layout::edit` (pure, arc 1);
//! the shell translates keys into them and owns the page.

use gridwatch_store::{KeyCode, KeyEvent};
use gridwatch_ui::layout::{Direction, EditError, Page, PlaceTarget, Placement};

/// Snapshots kept per direction (seam 4).
pub const UNDO_CAP: usize = 64;

/// The rect an op tried to occupy, in grid units; `ok` is the drag preview's
/// verdict (green fits, red does not).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ghost {
    pub at: (u8, u8),
    pub size: (u8, u8),
    pub ok: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickItem {
    /// What the picker shows: an instance id, or `kind:<name>`.
    pub label: String,
    pub target: PlaceTarget,
    /// The component kind, for the default footprint.
    pub kind: String,
}

#[derive(Clone, Debug, Default)]
pub struct Picker {
    pub items: Vec<PickItem>,
    pub filter: String,
    pub cursor: usize,
}

impl Picker {
    pub fn visible(&self) -> Vec<&PickItem> {
        self.items
            .iter()
            .filter(|i| self.filter.is_empty() || i.label.contains(&self.filter))
            .collect()
    }

    pub fn selected(&self) -> Option<PickItem> {
        self.visible().get(self.cursor).map(|i| (*i).clone())
    }

    pub fn down(&mut self) {
        let n = self.visible().len();
        if n > 0 {
            self.cursor = (self.cursor + 1) % n;
        }
    }

    pub fn up(&mut self) {
        let n = self.visible().len();
        if n > 0 {
            self.cursor = (self.cursor + n - 1) % n;
        }
    }

    pub fn type_char(&mut self, c: char) {
        self.filter.push(c);
        self.cursor = 0;
    }

    pub fn backspace(&mut self) {
        self.filter.pop();
        self.cursor = 0;
    }
}

/// A mouse drag in progress (seam 3): the placement, where it was pressed,
/// its original geometry, and whether the press was on the resize corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drag {
    pub index: usize,
    pub press: (u8, u8),
    pub origin_at: (u8, u8),
    pub origin_size: (u8, u8),
    pub resize: bool,
    pub last: (u8, u8),
}

impl Drag {
    /// The geometry the drag proposes at unit `cur`.
    pub fn proposed(&self, cur: (u8, u8)) -> ((u8, u8), (u8, u8)) {
        let dx = i16::from(cur.0) - i16::from(self.press.0);
        let dy = i16::from(cur.1) - i16::from(self.press.1);
        if self.resize {
            let w = (i16::from(self.origin_size.0) + dx).max(1) as u8;
            let h = (i16::from(self.origin_size.1) + dy).max(1) as u8;
            (self.origin_at, (w, h))
        } else {
            let x = (i16::from(self.origin_at.0) + dx).max(0) as u8;
            let y = (i16::from(self.origin_at.1) + dy).max(0) as u8;
            ((x, y), self.origin_size)
        }
    }
}

#[derive(Debug)]
pub struct EditState {
    /// The page as last loaded or saved: `dirty` compares against it.
    pub saved: Page,
    pub undo: Vec<Page>,
    pub redo: Vec<Page>,
    pub ghost: Option<Ghost>,
    /// The key-bar note (a refusal's reason) until the next key.
    pub note: Option<String>,
    /// `S` was pressed: the next `h/j/k/l` swaps.
    pub pending_swap: bool,
    /// `Esc` on a dirty page: `w` saves, `y` discards, `Esc` stays.
    pub confirm_leave: bool,
    /// A page change asked for while dirty: taken after `w` or `y`.
    pub pending_page: Option<usize>,
    pub picker: Option<Picker>,
    pub drag: Option<Drag>,
}

impl EditState {
    pub fn new(page: &Page) -> EditState {
        EditState {
            saved: page.clone(),
            undo: Vec::new(),
            redo: Vec::new(),
            ghost: None,
            note: None,
            pending_swap: false,
            confirm_leave: false,
            pending_page: None,
            picker: None,
            drag: None,
        }
    }

    pub fn dirty(&self, page: &Page) -> bool {
        *page != self.saved
    }

    /// A successful op: the old page goes on the undo stack, redo clears.
    pub fn commit(&mut self, page: &mut Page, next: Page) {
        self.undo.push(std::mem::replace(page, next));
        if self.undo.len() > UNDO_CAP {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.ghost = None;
        self.note = None;
    }

    /// A refused op: the page is unchanged; the ghost and the note say why.
    pub fn refuse(&mut self, err: EditError, at: (u8, u8), size: (u8, u8)) {
        self.ghost = Some(Ghost {
            at,
            size,
            ok: false,
        });
        self.note = Some(err.to_string());
    }

    pub fn undo(&mut self, page: &mut Page) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(page, prev));
                self.ghost = None;
                self.note = None;
                true
            }
            None => {
                self.note = Some("nothing to undo".into());
                false
            }
        }
    }

    pub fn redo(&mut self, page: &mut Page) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(page, next));
                self.ghost = None;
                self.note = None;
                true
            }
            None => {
                self.note = Some("nothing to redo".into());
                false
            }
        }
    }

    /// After a save: the page is the new baseline.
    pub fn mark_saved(&mut self, page: &Page) {
        self.saved = page.clone();
    }
}

/// The edit-mode keys (seam 1), decoded once so the shell's match is about
/// meaning. crossterm maps the bytes `0x08` and `0x0a` to `Ctrl-h` and
/// `Ctrl-j` itself (review: the earlier "Backspace/Enter arrive instead"
/// note was wrong), so only the ctrl spellings resize; the plain Backspace
/// and Return keys do nothing here, and `Delete` removes like `x`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKey {
    Move(i8, i8),
    Resize(i8, i8),
    Footprint,
    SwapPrefix,
    Dir(Direction),
    Picker,
    Remove,
    Undo,
    Redo,
    Save,
    Leave,
    Discard,
    Other,
}

pub fn decode(k: KeyEvent) -> EditKey {
    use KeyCode::*;
    if k.mods.ctrl {
        return match k.code {
            Char('l') | Right => EditKey::Resize(1, 0),
            Char('h') | Backspace | Left => EditKey::Resize(-1, 0),
            Char('j') | Enter | Down => EditKey::Resize(0, 1),
            Char('k') | Up => EditKey::Resize(0, -1),
            Char('r') => EditKey::Redo,
            _ => EditKey::Other,
        };
    }
    match k.code {
        Char('H') => EditKey::Move(-1, 0),
        Char('J') => EditKey::Move(0, 1),
        Char('K') => EditKey::Move(0, -1),
        Char('L') => EditKey::Move(1, 0),
        Char('h') | Left => EditKey::Dir(Direction::Left),
        Char('j') | Down => EditKey::Dir(Direction::Down),
        Char('k') | Up => EditKey::Dir(Direction::Up),
        Char('l') | Right => EditKey::Dir(Direction::Right),
        Char('s') => EditKey::Footprint,
        Char('S') => EditKey::SwapPrefix,
        Char('a') => EditKey::Picker,
        Char('x') | Delete => EditKey::Remove,
        Char('u') => EditKey::Undo,
        Char('w') => EditKey::Save,
        Char('y') => EditKey::Discard,
        Esc | Char('e') => EditKey::Leave,
        _ => EditKey::Other,
    }
}

/// The picker's items (seam 2): configured ids not on this page, then every
/// registered kind as `kind:<name>`.
pub fn picker_items<'a>(
    configured: impl Iterator<Item = (&'a str, &'a str)>,
    kinds: impl Iterator<Item = &'a str>,
    page: &Page,
) -> Vec<PickItem> {
    let on_page = |t: &PlaceTarget| page.place.iter().any(|p| p.target == *t);
    let mut items = Vec::new();
    for (id, kind) in configured {
        let target = PlaceTarget::Id(id.to_string());
        if !on_page(&target) {
            items.push(PickItem {
                label: id.to_string(),
                target,
                kind: kind.to_string(),
            });
        }
    }
    for k in kinds {
        items.push(PickItem {
            label: format!("kind:{k}"),
            target: PlaceTarget::Kind(k.to_string()),
            kind: k.to_string(),
        });
    }
    items
}

/// A fresh placement for a picked item at a footprint; `insert_first_fit`
/// picks the slot.
pub fn placement_for(item: &PickItem, footprint: (u8, u8)) -> Placement {
    Placement {
        target: item.target.clone(),
        at: (0, 0),
        size: footprint,
        view: None,
        priority: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridwatch_store::Mods;

    fn page(n: usize) -> Page {
        Page {
            name: "p".into(),
            hotkey: None,
            place: (0..n)
                .map(|i| Placement {
                    target: PlaceTarget::Id(format!("i{i}")),
                    at: (i as u8, 0),
                    size: (1, 1),
                    view: None,
                    priority: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn undo_redo_round_trip_and_cap() {
        let mut p = page(1);
        let mut st = EditState::new(&p);
        assert!(!st.dirty(&p));
        for i in 0..70 {
            let mut next = p.clone();
            next.place[0].at = (i % 5, 0);
            st.commit(&mut p, next);
        }
        assert_eq!(st.undo.len(), UNDO_CAP);
        assert!(st.dirty(&p));
        assert!(st.undo(&mut p));
        assert_eq!(p.place[0].at, (68 % 5, 0));
        assert!(st.redo(&mut p));
        assert_eq!(p.place[0].at, (69 % 5, 0));
        assert!(!st.redo(&mut p));
        assert_eq!(st.note.as_deref(), Some("nothing to redo"));
        st.mark_saved(&p);
        assert!(!st.dirty(&p));
    }

    #[test]
    fn keys_decode_both_spellings() {
        let ctrl = |c| KeyEvent {
            code: c,
            mods: Mods::CTRL,
        };
        assert_eq!(decode(ctrl(KeyCode::Char('l'))), EditKey::Resize(1, 0));
        assert_eq!(decode(ctrl(KeyCode::Char('h'))), EditKey::Resize(-1, 0));
        assert_eq!(decode(ctrl(KeyCode::Backspace)), EditKey::Resize(-1, 0));
        assert_eq!(decode(KeyEvent::plain(KeyCode::Backspace)), EditKey::Other);
        assert_eq!(decode(ctrl(KeyCode::Char('j'))), EditKey::Resize(0, 1));
        assert_eq!(decode(KeyEvent::plain(KeyCode::Enter)), EditKey::Other);
        assert_eq!(decode(KeyEvent::plain(KeyCode::Delete)), EditKey::Remove);
        assert_eq!(decode(ctrl(KeyCode::Char('k'))), EditKey::Resize(0, -1));
        assert_eq!(decode(KeyEvent::ch('H')), EditKey::Move(-1, 0));
        assert_eq!(decode(KeyEvent::ch('l')), EditKey::Dir(Direction::Right));
        assert_eq!(decode(ctrl(KeyCode::Char('r'))), EditKey::Redo);
        assert_eq!(decode(KeyEvent::ch('e')), EditKey::Leave);
    }

    #[test]
    fn picker_lists_unplaced_ids_then_kinds_and_filters() {
        let p = page(1);
        let items = picker_items(
            [("i0", "htop"), ("gpu", "gpu")].into_iter(),
            ["clock", "gpu"].into_iter(),
            &p,
        );
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["gpu", "kind:clock", "kind:gpu"]);
        let mut pk = Picker {
            items,
            ..Picker::default()
        };
        pk.type_char('c');
        assert_eq!(pk.visible().len(), 1);
        assert_eq!(pk.selected().unwrap().kind, "clock");
        pk.backspace();
        pk.up();
        assert_eq!(pk.selected().unwrap().label, "kind:gpu");
    }

    #[test]
    fn drag_proposes_move_or_resize() {
        let d = Drag {
            index: 0,
            press: (3, 2),
            origin_at: (2, 1),
            origin_size: (4, 2),
            resize: false,
            last: (3, 2),
        };
        assert_eq!(d.proposed((5, 3)), ((4, 2), (4, 2)));
        assert_eq!(
            d.proposed((0, 0)),
            ((0, 0), (4, 2)),
            "clamped at the origin"
        );
        let r = Drag { resize: true, ..d };
        assert_eq!(r.proposed((5, 1)), ((2, 1), (6, 1)));
        assert_eq!(r.proposed((0, 0)), ((2, 1), (1, 1)), "never below one unit");
    }

    proptest::proptest! {
        /// Seam 3 / ROADMAP: the mouse paths never produce an overlapping or
        /// out-of-grid page — whatever the press/drag units, the op either
        /// yields a valid page or is refused.
        #[test]
        fn drag_ops_never_break_the_grid(
            px in 0u8..12, py in 0u8..6, cx in 0u8..14, cy in 0u8..8, resize in proptest::bool::ANY,
        ) {
            use gridwatch_ui::layout::{GridSpec, move_by, resize_by};
            let spec = GridSpec::default();
            let mut page = page(2);
            page.place[0].size = (4, 2);
            page.place[1].at = (6, 0);
            page.place[1].size = (3, 3);
            let d = Drag { index: 0, press: (px, py), origin_at: (0, 0), origin_size: (4, 2), resize, last: (px, py) };
            let (at, size) = d.proposed((cx, cy));
            let r = if resize {
                resize_by(&spec, &page, 0, (i16::from(size.0) - 4) as i8, (i16::from(size.1) - 2) as i8)
            } else {
                move_by(&spec, &page, 0, i16::from(at.0) as i8, i16::from(at.1) as i8)
            };
            if let Ok(next) = r {
                for (i, a) in next.place.iter().enumerate() {
                    proptest::prop_assert!(a.in_bounds(spec.columns, spec.rows));
                    for (j, b) in next.place.iter().enumerate() {
                        proptest::prop_assert!(i == j || !a.overlaps(b));
                    }
                }
            }
        }
    }
}
