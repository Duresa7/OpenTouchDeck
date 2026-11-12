//! Types the deck's mirrored key reports on the PC.
//!
//! The release guarantee: whatever reports go missing, nothing stays held. A `seq` gap releases
//! every held key before the next report applies, `key.resync` sets the held state exactly, and
//! dropping the session, or the [`Mirror`], releases everything.

use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::Value;

use crate::keymap::{self, Code, modifier_usage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    Down(Code),
    Up(Code),
}

/// Turns key events into operating system input.
pub trait InputSink: Send {
    fn send(&mut self, event: KeyEvent) -> Result<(), String>;
}

/// A sink that records events instead of typing them, for tests and for Linux without uinput.
#[derive(Debug, Clone, Default)]
pub struct FakeSink {
    pub events: Arc<Mutex<Vec<KeyEvent>>>,
}

impl FakeSink {
    pub fn take(&self) -> Vec<KeyEvent> {
        std::mem::take(&mut self.events.lock().unwrap())
    }
}

impl InputSink for FakeSink {
    fn send(&mut self, event: KeyEvent) -> Result<(), String> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

/// The operating system's input: `SendInput` on Windows, `uinput` on Linux.
pub fn system_sink() -> Result<Box<dyn InputSink>, String> {
    #[cfg(windows)]
    return Ok(Box::new(crate::input_windows::SendInputSink));
    #[cfg(target_os = "linux")]
    return crate::input_linux::UinputSink::open().map(|sink| Box::new(sink) as Box<dyn InputSink>);
    #[cfg(not(any(windows, target_os = "linux")))]
    Err("typing keys is not supported on this platform".into())
}

/// A mirrored event as the deck sends it: `key.report` or, after its queue overflowed, `key.resync`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "m")]
pub enum Mirrored {
    #[serde(rename = "key.report")]
    Report {
        seq: u32,
        #[serde(rename = "type")]
        kind: String,
        #[serde(default)]
        mods: u8,
        #[serde(default)]
        keys: Vec<u8>,
        #[serde(default)]
        usage: u16,
    },
    #[serde(rename = "key.resync")]
    Resync {
        seq: u32,
        #[serde(default)]
        mods: u8,
        #[serde(default)]
        keys: Vec<u8>,
    },
}

impl Mirrored {
    pub fn parse(event: &Value) -> Option<Mirrored> {
        match event.get("m").and_then(Value::as_str) {
            Some("key.report" | "key.resync") => serde_json::from_value(event.clone()).ok(),
            _ => None,
        }
    }

    pub fn seq(&self) -> u32 {
        match self {
            Mirrored::Report { seq, .. } | Mirrored::Resync { seq, .. } => *seq,
        }
    }

    /// The report as the key monitor lists it, such as `kbd 0 [104]` or `con 233`.
    pub fn describe(&self) -> String {
        let keys = |keys: &[u8]| {
            let list: Vec<String> = keys.iter().map(u8::to_string).collect();
            format!("[{}]", list.join(","))
        };
        match self {
            Mirrored::Report {
                kind,
                mods,
                keys: k,
                ..
            } if kind == "kbd" => format!("kbd {mods} {}", keys(k)),
            Mirrored::Report { kind, usage, .. } => format!("{kind} {usage}"),
            Mirrored::Resync { mods, keys: k, .. } => format!("resync {mods} {}", keys(k)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Typed. `gap` names the sequence numbers that never arrived, whose held keys were released.
    Typed {
        gap: Option<(u32, u32)>,
    },
    Duplicate,
    /// Mirroring is off, because the deck types through its own HID sink.
    Off,
    Unknown,
}

/// The mirror's held state and the sink it types into.
pub struct Mirror {
    sink: Box<dyn InputSink>,
    enabled: bool,
    mods: u8,
    keys: Vec<u8>,
    last_seq: Option<u32>,
    pub errors: Vec<String>,
}

impl Mirror {
    pub fn new(sink: Box<dyn InputSink>) -> Mirror {
        Mirror {
            sink,
            enabled: true,
            mods: 0,
            keys: Vec::new(),
            last_seq: None,
            errors: Vec::new(),
        }
    }

    /// Mirroring stays off while the deck reports a `hid` sink, so the PC never gets a key twice.
    pub fn wanted(sinks: &[String]) -> bool {
        !sinks.iter().any(|sink| sink == "hid")
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if !enabled {
            self.release_all();
        }
        self.enabled = enabled;
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn held(&self) -> (u8, &[u8]) {
        (self.mods, &self.keys)
    }

    pub fn apply(&mut self, event: &Mirrored) -> Outcome {
        if !self.enabled {
            return Outcome::Off;
        }
        let seq = event.seq();
        if let Mirrored::Resync { mods, keys, .. } = event {
            self.last_seq = Some(seq);
            self.set_state(*mods, keys);
            return Outcome::Typed { gap: None };
        }
        let mut gap = None;
        if let Some(last) = self.last_seq {
            if seq <= last {
                return Outcome::Duplicate;
            }
            if seq - last > 1 {
                gap = Some((last + 1, seq - 1));
                self.release_all();
            }
        }
        self.last_seq = Some(seq);
        let Mirrored::Report {
            kind,
            mods,
            keys,
            usage,
            ..
        } = event
        else {
            unreachable!("resync returned above");
        };
        match kind.as_str() {
            "kbd" => self.set_state(*mods, keys),
            "con" if *usage != 0 => {
                self.emit(KeyEvent::Down(Code::Consumer(*usage)));
                self.emit(KeyEvent::Up(Code::Consumer(*usage)));
            }
            "con" => {}
            _ => return Outcome::Unknown,
        }
        Outcome::Typed { gap }
    }

    /// Releases every held key, the keys in reverse order and then the modifiers.
    pub fn release_all(&mut self) {
        self.set_state(0, &[]);
    }

    /// Moves the held state to exactly `mods` and `keys`. Keys that leave are released in
    /// reverse, then modifiers that leave; modifiers that join are pressed, then keys that join.
    fn set_state(&mut self, mods: u8, keys: &[u8]) {
        let target: Vec<u8> = keys.iter().copied().filter(|&key| key != 0).collect();
        let leaving: Vec<u8> = self
            .keys
            .iter()
            .rev()
            .copied()
            .filter(|key| !target.contains(key))
            .collect();
        for key in leaving {
            self.emit(KeyEvent::Up(Code::Keyboard(key)));
        }
        for bit in (0..8).rev() {
            if self.mods & !mods & (1 << bit) != 0 {
                self.emit(KeyEvent::Up(Code::Keyboard(modifier_usage(bit))));
            }
        }
        for bit in 0..8 {
            if mods & !self.mods & (1 << bit) != 0 {
                self.emit(KeyEvent::Down(Code::Keyboard(modifier_usage(bit))));
            }
        }
        for &key in &target {
            if !self.keys.contains(&key) {
                self.emit(KeyEvent::Down(Code::Keyboard(key)));
            }
        }
        self.mods = mods;
        self.keys = target;
    }

    fn emit(&mut self, event: KeyEvent) {
        if let Err(error) = self.sink.send(event) {
            let code = match event {
                KeyEvent::Down(code) | KeyEvent::Up(code) => code,
            };
            self.errors.push(format!("{}: {error}", keymap::name(code)));
        }
    }
}

impl Drop for Mirror {
    fn drop(&mut self) {
        self.release_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use KeyEvent::{Down, Up};
    use serde_json::json;

    const CTRL: Code = Code::Keyboard(0xE0);
    const SHIFT: Code = Code::Keyboard(0xE1);
    const F13: Code = Code::Keyboard(104);
    const F15: Code = Code::Keyboard(106);

    fn mirror() -> (Mirror, FakeSink) {
        let fake = FakeSink::default();
        (Mirror::new(Box::new(fake.clone())), fake)
    }

    fn kbd(seq: u32, mods: u8, keys: &[u8]) -> Mirrored {
        Mirrored::parse(&json!({"m": "key.report", "seq": seq, "t": 0, "type": "kbd", "mods": mods, "keys": keys}))
            .unwrap()
    }

    fn con(seq: u32, usage: u16) -> Mirrored {
        Mirrored::parse(
            &json!({"m": "key.report", "seq": seq, "t": 0, "type": "con", "usage": usage}),
        )
        .unwrap()
    }

    #[test]
    fn ctrl_f15_presses_in_order_and_releases_in_reverse() {
        let (mut mirror, fake) = mirror();
        mirror.apply(&kbd(1, 1, &[106]));
        mirror.apply(&kbd(2, 0, &[]));
        assert_eq!(fake.take(), [Down(CTRL), Down(F15), Up(F15), Up(CTRL)]);
        assert_eq!(mirror.held(), (0, &[][..]));
    }

    #[test]
    fn a_consumer_report_taps_its_key_and_the_empty_one_does_nothing() {
        let (mut mirror, fake) = mirror();
        mirror.apply(&con(1, 233));
        mirror.apply(&con(2, 0));
        let up = Code::Consumer(233);
        assert_eq!(fake.take(), [Down(up), Up(up)]);
    }

    #[test]
    fn a_gap_releases_only_what_was_pressed_and_is_never_replayed() {
        let (mut mirror, fake) = mirror();
        mirror.apply(&kbd(1, 2, &[106]));
        fake.take();
        assert_eq!(
            mirror.apply(&kbd(5, 0, &[104])),
            Outcome::Typed { gap: Some((2, 4)) }
        );
        assert_eq!(fake.take(), [Up(F15), Up(SHIFT), Down(F13)]);
    }

    #[test]
    fn a_gap_with_nothing_held_releases_nothing() {
        let (mut mirror, fake) = mirror();
        mirror.apply(&con(1, 233));
        fake.take();
        mirror.apply(&con(4, 0));
        assert_eq!(fake.take(), []);
    }

    #[test]
    fn a_dropped_release_followed_by_a_consumer_stream_leaves_nothing_down() {
        let (mut mirror, fake) = mirror();
        mirror.apply(&kbd(1, 1, &[106]));
        // seq 2, the release, never arrives.
        mirror.apply(&con(3, 233));
        mirror.apply(&con(4, 0));
        mirror.apply(&con(5, 234));
        mirror.apply(&con(6, 0));
        let (up, down) = (Code::Consumer(233), Code::Consumer(234));
        assert_eq!(
            fake.take(),
            [
                Down(CTRL),
                Down(F15),
                Up(F15),
                Up(CTRL),
                Down(up),
                Up(up),
                Down(down),
                Up(down)
            ]
        );
        assert_eq!(mirror.held(), (0, &[][..]));
    }

    #[test]
    fn a_duplicate_is_dropped() {
        let (mut mirror, fake) = mirror();
        mirror.apply(&kbd(1, 0, &[104]));
        assert_eq!(mirror.apply(&kbd(1, 0, &[104])), Outcome::Duplicate);
        assert_eq!(fake.take(), [Down(F13)]);
    }

    #[test]
    fn a_resync_sets_the_state_exactly() {
        let (mut mirror, fake) = mirror();
        mirror.apply(&kbd(1, 3, &[104, 106]));
        fake.take();
        let resync = Mirrored::parse(
            &json!({"m": "key.resync", "seq": 9, "t": 0, "mods": 2, "keys": [106]}),
        )
        .unwrap();
        mirror.apply(&resync);
        assert_eq!(fake.take(), [Up(F13), Up(CTRL)]);
        assert_eq!(mirror.held(), (2, &[106u8][..]));
        mirror.apply(&kbd(10, 0, &[]));
        assert_eq!(fake.take(), [Up(F15), Up(SHIFT)]);
    }

    #[test]
    fn dropping_the_mirror_releases_held_keys() {
        let (mut mirror, fake) = mirror();
        mirror.apply(&kbd(1, 1, &[104]));
        fake.take();
        drop(mirror);
        assert_eq!(fake.take(), [Up(F13), Up(CTRL)]);
    }

    #[test]
    fn turning_mirroring_off_releases_and_types_nothing_more() {
        let (mut mirror, fake) = mirror();
        mirror.apply(&kbd(1, 0, &[104]));
        mirror.set_enabled(false);
        assert_eq!(mirror.apply(&kbd(2, 0, &[105])), Outcome::Off);
        assert_eq!(fake.take(), [Down(F13), Up(F13)]);
    }

    #[test]
    fn bluetooth_keeps_mirroring_on_and_usb_hid_turns_it_off() {
        assert!(Mirror::wanted(&[
            "log".into(),
            "link".into(),
            "bluetooth".into()
        ]));
        assert!(Mirror::wanted(&["log".into(), "link".into()]));
        assert!(!Mirror::wanted(&[
            "log".into(),
            "link".into(),
            "hid".into()
        ]));
    }

    #[test]
    fn reports_describe_as_the_monitor_lists_them() {
        assert_eq!(kbd(1, 0, &[104]).describe(), "kbd 0 [104]");
        assert_eq!(kbd(1, 2, &[]).describe(), "kbd 2 []");
        assert_eq!(con(1, 233).describe(), "con 233");
        assert_eq!(Mirrored::parse(&json!({"m": "press"})), None);
    }
}
