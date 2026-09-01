//! Journal stub (§4.5). Record/replay lands in arc 2; the error type exists now
//! because the key catalogue's `decode` entries (§4.1) are part of the arc-1 seam.

use std::fmt;

#[derive(Debug)]
pub struct JournalError(pub String);

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "journal: {}", self.0)
    }
}

impl std::error::Error for JournalError {}
