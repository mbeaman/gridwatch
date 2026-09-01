//! Messages and the three channels (§4.2): data is bounded and lossy with a
//! drop counter; control and input are unbounded and never dropped. The frame
//! loop drains input → control → data.

use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};

use crate::alert::AlertEvent;
use crate::input::InputEvent;
use crate::key::{Datum, MetricId};
use crate::source::{SourceId, SourceStatus};
use crate::ts::Ts;

#[derive(Clone, Debug)]
pub struct Sample {
    pub id: MetricId,
    pub datum: Datum,
}

#[derive(Clone, Debug)]
pub struct Batch {
    pub source: SourceId,
    pub at: Ts,
    pub samples: Vec<Sample>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActionId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadKind {
    Config,
    Layout,
    Theme,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reload {
    pub kind: ReloadKind,
}

#[derive(Debug)]
pub enum ControlMsg {
    Status(SourceId, SourceStatus),
    Alert(AlertEvent),
    Done(ActionId, Result<String, String>),
    Reload(Reload),
}

#[derive(Debug)]
pub enum Msg {
    Batch(Batch),
    Control(ControlMsg),
    Input(InputEvent),
    Heartbeat,
}

/// Bound of the lossy data channel; a 60 Hz publisher plus every poller cannot
/// fill it in under ~40 s of a stalled render thread (§11).
pub const DATA_BOUND: usize = 4096;

/// The senders handed to sources (and the input/watcher threads).
#[derive(Clone)]
pub struct Channels {
    pub data: SyncSender<Batch>,
    pub control: Sender<ControlMsg>,
    pub input: Sender<InputEvent>,
}

/// The receivers owned by the render thread.
pub struct Inbox {
    pub data: Receiver<Batch>,
    pub control: Receiver<ControlMsg>,
    pub input: Receiver<InputEvent>,
}

pub fn channels() -> (Channels, Inbox) {
    let (data_tx, data_rx) = sync_channel(DATA_BOUND);
    let (control_tx, control_rx) = channel();
    let (input_tx, input_rx) = channel();
    (
        Channels {
            data: data_tx,
            control: control_tx,
            input: input_tx,
        },
        Inbox {
            data: data_rx,
            control: control_rx,
            input: input_rx,
        },
    )
}
