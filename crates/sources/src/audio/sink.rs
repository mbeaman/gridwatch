//! Sink enumeration over `pw-dump` JSON (digest §1a): `PipeWire:Interface:Node`
//! objects with `media.class == "Audio/Sink"` and the `default` metadata's
//! `default.audio.sink`. `object.serial` is the only stable target id —
//! never the node id (`wpctl status` prints ids).

use std::process::Command;
use std::time::Duration;

use gridwatch_store::keys::audio::{AudioSink, AudioSinks};
use serde_json::Value;

/// How often the list is refreshed while a picker is open.
pub const ENUMERATE_EVERY: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dump {
    pub sinks: Vec<AudioSink>,
    /// `default.audio.sink` → `node.name`.
    pub default: Option<String>,
}

impl Dump {
    /// The sink a target resolves to: `auto` = the default, a serial or a name.
    pub fn resolve(&self, target: &super::capture::Target) -> Option<&AudioSink> {
        use super::capture::Target;
        match target {
            Target::Auto => self
                .sinks
                .iter()
                .find(|s| s.is_default)
                .or_else(|| self.sinks.first()),
            Target::Serial(n) => self.sinks.iter().find(|s| s.serial == *n),
            Target::Name(n) => self.sinks.iter().find(|s| &s.name == n),
        }
    }

    pub fn record(&self) -> AudioSinks {
        AudioSinks {
            sinks: self.sinks.clone(),
        }
    }
}

fn prop<'a>(props: &'a Value, key: &str) -> Option<&'a Value> {
    props.get(key)
}

/// Parse a `pw-dump` document.
pub fn parse_dump(json: &str) -> Result<Dump, String> {
    let root: Value = serde_json::from_str(json).map_err(|e| format!("pw-dump JSON: {e}"))?;
    let objects = root
        .as_array()
        .ok_or_else(|| "pw-dump: not an array".to_string())?;
    let mut out = Dump::default();
    for o in objects {
        let ty = o.get("type").and_then(Value::as_str).unwrap_or("");
        let info = o.get("info").unwrap_or(&Value::Null);
        // Nodes carry their props under `info`; a Metadata object carries
        // them at the top level (verified on torch's pw-dump, 1.6.2).
        let props = info
            .get("props")
            .or_else(|| o.get("props"))
            .unwrap_or(&Value::Null);
        match ty {
            "PipeWire:Interface:Node" => {
                if prop(props, "media.class").and_then(Value::as_str) != Some("Audio/Sink") {
                    continue;
                }
                let name = prop(props, "node.name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                let description = prop(props, "node.description")
                    .or_else(|| prop(props, "node.nick"))
                    .and_then(Value::as_str)
                    .unwrap_or(&name)
                    .to_string();
                let serial = prop(props, "object.serial")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32;
                let state = info
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                let rate = prop(props, "audio.rate")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32;
                let channels = prop(props, "audio.channels")
                    .and_then(Value::as_u64)
                    .unwrap_or(2) as u8;
                out.sinks.push(AudioSink {
                    name,
                    description,
                    serial,
                    state,
                    is_default: false,
                    rate,
                    channels,
                });
            }
            "PipeWire:Interface:Metadata" => {
                if prop(props, "metadata.name").and_then(Value::as_str) != Some("default") {
                    continue;
                }
                if let Some(items) = o.get("metadata").and_then(Value::as_array) {
                    for it in items {
                        if it.get("key").and_then(Value::as_str) == Some("default.audio.sink") {
                            out.default = it
                                .get("value")
                                .and_then(|v| v.get("name"))
                                .and_then(Value::as_str)
                                .map(String::from);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(d) = out.default.clone() {
        for s in &mut out.sinks {
            s.is_default = s.name == d;
        }
    }
    Ok(out)
}

/// Run `pw-dump` (≈ 10 ms, 280 KB) and parse it.
pub fn enumerate() -> Result<Dump, String> {
    let out = Command::new("pw-dump")
        .output()
        .map_err(|e| format!("pw-dump: {e}"))?;
    if !out.status.success() {
        return Err(format!("pw-dump exited {}", out.status));
    }
    parse_dump(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(test)]
pub(crate) const EXCERPT: &str = r#"[
  { "id": 61, "type": "PipeWire:Interface:Node", "version": 3,
    "info": { "state": "running",
      "props": { "object.serial": 61, "media.class": "Audio/Sink",
        "node.name": "alsa_output.usb-Generic_USB_Audio-00.HiFi__Headphones__sink",
        "node.description": "USB Audio Headphones", "audio.rate": 48000, "audio.channels": 2 } } },
  { "id": 75, "type": "PipeWire:Interface:Node", "version": 3,
    "info": { "state": "suspended",
      "props": { "object.serial": 366, "media.class": "Audio/Sink",
        "node.name": "alsa_output.pci-0000_01_00.1.hdmi-stereo",
        "node.nick": "HDMI" } } },
  { "id": 80, "type": "PipeWire:Interface:Node", "version": 3,
    "info": { "state": "running",
      "props": { "object.serial": 80, "media.class": "Stream/Output/Audio", "node.name": "firefox" } } },
  { "id": 33, "type": "PipeWire:Interface:Metadata", "version": 3,
    "props": { "metadata.name": "default" },
    "metadata": [
      { "subject": 0, "key": "default.configured.audio.sink", "type": "Spa:String:JSON", "value": { "name": "alsa_output.pci-0000_0d_00.4.analog-stereo" } },
      { "subject": 0, "key": "default.audio.sink", "type": "Spa:String:JSON", "value": { "name": "alsa_output.usb-Generic_USB_Audio-00.HiFi__Headphones__sink" } },
      { "subject": 0, "key": "default.audio.source", "type": "Spa:String:JSON", "value": { "name": "alsa_output.usb-Generic_USB_Audio-00.HiFi__Headphones__sink" } }
    ] }
]"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::capture::Target;

    #[test]
    fn parses_sinks_serials_state_and_the_default() {
        let d = parse_dump(EXCERPT).unwrap();
        assert_eq!(d.sinks.len(), 2, "streams are not sinks");
        let usb = &d.sinks[0];
        assert_eq!(usb.serial, 61);
        assert_eq!(usb.state, "running");
        assert!(usb.is_default);
        assert_eq!(usb.description, "USB Audio Headphones");
        assert_eq!(usb.rate, 48_000);
        let hdmi = &d.sinks[1];
        assert_eq!(hdmi.serial, 366, "serial, never the node id 75");
        assert_eq!(hdmi.description, "HDMI", "nick when no description");
        assert!(!hdmi.is_default);
        assert_eq!(hdmi.state, "suspended");
        assert_eq!(
            d.default.as_deref(),
            Some("alsa_output.usb-Generic_USB_Audio-00.HiFi__Headphones__sink"),
            "default.audio.sink, not the configured one"
        );
        assert_eq!(d.resolve(&Target::Auto).unwrap().serial, 61);
        assert_eq!(d.resolve(&Target::Serial(366)).unwrap().description, "HDMI");
        assert!(
            d.resolve(&Target::Serial(75)).is_none(),
            "a node id resolves nothing"
        );
        assert_eq!(
            d.resolve(&Target::Name(
                "alsa_output.pci-0000_01_00.1.hdmi-stereo".into()
            ))
            .unwrap()
            .serial,
            366
        );
        assert_eq!(d.record().sinks.len(), 2);
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert!(parse_dump("{").is_err());
        assert!(parse_dump("{}").is_err());
        assert_eq!(parse_dump("[]").unwrap(), Dump::default());
        assert_eq!(
            parse_dump(r#"[{"type":"PipeWire:Interface:Node"}]"#)
                .unwrap()
                .sinks
                .len(),
            0
        );
    }
}
