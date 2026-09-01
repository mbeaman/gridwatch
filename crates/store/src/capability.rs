//! Host capabilities (§4.6, probe lives in the app crate §11).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    Procfs,
    Hwmon,
    Cpufreq,
    Rapl,
    Nvml,
    I2cNvidia,
    AstralExporter,
    AstralCsv,
    PwRecord,
    PipeWireSocket,
    DbusSession,
    PingSocket,
    NetRaw,
    TrueColor,
    VteGlyphs,
    Mouse,
}

pub const ALL_CAPABILITIES: &[Capability] = &[
    Capability::Procfs,
    Capability::Hwmon,
    Capability::Cpufreq,
    Capability::Rapl,
    Capability::Nvml,
    Capability::I2cNvidia,
    Capability::AstralExporter,
    Capability::AstralCsv,
    Capability::PwRecord,
    Capability::PipeWireSocket,
    Capability::DbusSession,
    Capability::PingSocket,
    Capability::NetRaw,
    Capability::TrueColor,
    Capability::VteGlyphs,
    Capability::Mouse,
];

fn bit(c: Capability) -> u32 {
    1 << ALL_CAPABILITIES
        .iter()
        .position(|x| *x == c)
        .expect("capability listed") as u32
}

/// A set of probed capabilities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CapSet(u32);

impl CapSet {
    pub fn empty() -> CapSet {
        CapSet(0)
    }

    pub fn insert(&mut self, c: Capability) {
        self.0 |= bit(c);
    }

    pub fn has(&self, c: Capability) -> bool {
        self.0 & bit(c) != 0
    }

    pub fn has_all(&self, cs: &[Capability]) -> bool {
        cs.iter().all(|c| self.has(*c))
    }

    pub fn missing(&self, cs: &[Capability]) -> Vec<Capability> {
        cs.iter().copied().filter(|c| !self.has(*c)).collect()
    }
}

impl FromIterator<Capability> for CapSet {
    fn from_iter<I: IntoIterator<Item = Capability>>(iter: I) -> CapSet {
        let mut s = CapSet::empty();
        for c in iter {
            s.insert(c);
        }
        s
    }
}
