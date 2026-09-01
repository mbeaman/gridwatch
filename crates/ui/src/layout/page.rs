//! Pages and placements (§6, §9): fixed grid units, solved per terminal size.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaceTarget {
    /// A configured component instance id.
    Id(String),
    /// An anonymous default-options instance of a kind (§9 `kind = "clock"`).
    Kind(String),
}

impl PlaceTarget {
    pub fn label(&self) -> &str {
        match self {
            PlaceTarget::Id(s) | PlaceTarget::Kind(s) => s,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub target: PlaceTarget,
    pub at: (u8, u8),
    pub size: (u8, u8),
    pub view: Option<String>,
    pub priority: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub name: String,
    pub hotkey: Option<char>,
    pub place: Vec<Placement>,
}

impl Placement {
    pub fn overlaps(&self, other: &Placement) -> bool {
        let (ax0, ay0) = (self.at.0, self.at.1);
        let (ax1, ay1) = (ax0 + self.size.0, ay0 + self.size.1);
        let (bx0, by0) = (other.at.0, other.at.1);
        let (bx1, by1) = (bx0 + other.size.0, by0 + other.size.1);
        ax0 < bx1 && bx0 < ax1 && ay0 < by1 && by0 < ay1
    }

    pub fn in_bounds(&self, columns: u8, rows: u8) -> bool {
        // u16 math: `at + size` may exceed u8::MAX and must fail, not wrap.
        self.size.0 >= 1
            && self.size.1 >= 1
            && u16::from(self.at.0) + u16::from(self.size.0) <= u16::from(columns)
            && u16::from(self.at.1) + u16::from(self.size.1) <= u16::from(rows)
    }
}
