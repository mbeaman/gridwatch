//! The `exec` plugin host (§4.7, D39/D58 seam 7, arc 8b).
//!
//! One child process per plugin, JSON lines over its stdin and stdout,
//! supervised the way a source is. What makes this safe to point at a
//! stranger's program is not trust, it is shape:
//!
//! * **No shell.** `Command::new(argv[0]).args(&argv[1..])` — never a
//!   string handed to `sh -c`, so a plugin path with a space in it is a
//!   path, not two words, and nothing in a config file is ever
//!   interpreted.
//! * **Every line is validated before it is read**, and the reader is
//!   bounded: a line over `MAX_LINE` is refused without allocating it.
//! * **Three strikes.** Three malformed messages stop the plugin instead
//!   of restarting it: a plugin that cannot speak the protocol will not
//!   start speaking it after a fourth try, and a restart loop is how a
//!   broken plugin becomes a busy machine.
//! * **A plugin returns a view tree, never cells.** It cannot choose a
//!   colour, write outside its rect, or reach a device.
//! * **Caps.** Memory and CPU limits on the child, and a supervisor that
//!   stops one that outgrows them.

pub mod host;
pub mod proto;
pub mod supervise;
pub mod tile;
pub mod tree;

pub use host::{Host, Started, Word};
pub use supervise::{Plugin, PluginConfig, Report};
pub use tile::PluginTile;
