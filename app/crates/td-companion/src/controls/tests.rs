use super::*;
use crate::actions::{Actions, AllowedAction, GrantStore};
use crate::devices::{Config, DeviceManager, LinkState};
use crate::integrations::Service;
use crate::link::Links;
use crate::live::{Live, fake::FakeControl};
use crate::{api::Events, clock::Clock};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Instant;
use td_core::InputPhase;
use td_host::integrations::Control;
use td_link::MockDeck;

const DEVICE: &str = "mock0001";
const MEDIA: &[u8] = include_bytes!("../../../../../schema/examples/media.json");

struct NoGrants;
impl GrantStore for NoGrants {
    fn load(&self) -> std::io::Result<Vec<AllowedAction>> {
        Ok(Vec::new())
    }
    fn save(&self, _: &[AllowedAction]) -> std::io::Result<()> {
        Ok(())
    }
}

struct Running {
    actions: Actions,
    deck: MockDeck,
    devices: DeviceManager,
    live: Option<Live>,
    authority: LayoutAuthority,
    volume: Arc<FakeControl>,
    media: Arc<FakeControl>,
    volume_on: Arc<std::sync::atomic::AtomicBool>,
    _data: tempfile::TempDir,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.live.take();
        self.devices.stop();
    }
}

type Entry = (String, String, String, String);

impl Running {
    fn log(&self) -> Vec<Entry> {
        self.actions
            .log(None)
            .into_iter()
            .map(|l| (l.tile, l.kind, l.status, l.detail))
            .collect()
    }

    fn drag(&self, tile: &str, values: &[f64]) {
        for (i, value) in values.iter().enumerate() {
            let phase = if i + 1 == values.len() {
                InputPhase::End
            } else {
                InputPhase::Move
            };
            self.deck.emit_input("media", tile, "value", phase, *value);
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    fn last_set(&self) -> Option<f64> {
        self.volume.sets.lock().last().map(|set| set.2)
    }
}

fn wait(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn media() -> Value {
    serde_json::from_slice(MEDIA).unwrap()
}

fn running(layout: Value, cached: bool) -> Running {
    running_with_authority(layout, cached, cached)
}

fn running_with_authority(layout: Value, cached: bool, pushed: bool) -> Running {
    let events = Events::default();
    let devices = DeviceManager::new(events.clone());
    let config = Config::mock(None);
    let ports = config.mock.as_ref().unwrap().0.clone();
    ports.plug_media_device(DEVICE);
    let deck = ports.deck(DEVICE).unwrap();
    devices.start(config);
    let id = DeviceId::parse(DEVICE).unwrap();
    let session = || match devices.get(&id).map(|d| d.link) {
        Some(LinkState::Connected { since_ms, .. }) => Some(since_ms),
        _ => None,
    };
    wait("the Device connects", || session().is_some());
    let links = Links::new(devices.clone());
    let layout = td_core::Layout::parse(&serde_json::to_vec(&layout).unwrap()).unwrap();
    links
        .with(DEVICE, |s| {
            td_link::xfer::put_layout(s, layout.orientation, &layout)
        })
        .unwrap()
        .unwrap();
    let volume = Arc::new(FakeControl::default());
    let media = Arc::new(FakeControl::default());
    let controls: Controls = Arc::new(BTreeMap::from([
        ("volume".to_owned(), volume.clone() as Arc<dyn Control>),
        ("media".to_owned(), media.clone() as Arc<dyn Control>),
    ]));
    let mut service = Service::new(true, events.clone());
    service.controls = controls.clone();
    let watched = if cached {
        devices.clone()
    } else {
        DeviceManager::new(events.clone())
    };

    let actions = Actions::new(
        events,
        Clock::default(),
        devices.clone(),
        links.clone(),
        Arc::new(NoGrants),
        true,
    );
    let volume_on = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let turned_on = volume_on.clone();
    let data = tempfile::tempdir().unwrap();
    let layouts = crate::store::LayoutStore::new(data.path().join("layouts"));
    if pushed {
        layouts.put(DEVICE, &layout).unwrap();
    }
    let settable: Settable = Arc::new(|key| key.integration == "volume" && key.stream == "level");
    let authority = LayoutAuthority::new(layouts, settable.clone(), watched.clone());
    let live = Live::start(watched, links.clone(), service, authority.clone());
    actions.attach(
        authority.clone(),
        controls,
        settable,
        Arc::new(move |id| id != "volume" || turned_on.load(std::sync::atomic::Ordering::SeqCst)),
    );
    if cached && pushed {
        wait("the router reads the Layout", || {
            authority
                .tile(DEVICE, session().unwrap(), "media", "volume")
                .is_some()
        });
    }
    Running {
        actions,
        deck,
        devices,
        live: Some(live),
        authority,
        _data: data,
        volume,
        media,
        volume_on,
    }
}

fn entry(tile: &str, status: &str, detail: &str) -> Entry {
    (
        tile.into(),
        "integration".into(),
        status.into(),
        detail.into(),
    )
}

#[test]
fn a_drag_sets_the_bound_integration_stream_and_logs_once_per_gesture() {
    let td = running(media(), true);
    td.drag("volume", &[0.2, 0.4, 0.5, 0.6]);
    wait("the gesture ends", || td.last_set() == Some(0.6));
    let sets = td.volume.sets.lock().clone();
    assert!(
        sets.iter()
            .all(|(account, stream, value)| account.is_empty()
                && stream == "level"
                && [0.2, 0.4, 0.5, 0.6].contains(value)),
        "{sets:?}"
    );
    wait("the gesture logs", || !td.log().is_empty());
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(td.log(), [entry("volume", "ok", GESTURE_DONE)]);
}

#[test]
fn a_drag_in_progress_stops_when_its_integration_is_turned_off_and_logs_no_failure() {
    let td = running(media(), true);
    td.deck
        .emit_input("media", "volume", "value", InputPhase::Move, 0.3);
    wait("the first move is set", || td.last_set() == Some(0.3));
    *td.volume.set_result.lock() = Some(ControlError::Unavailable("Volume is unavailable.".into()));
    td.deck
        .emit_input("media", "volume", "value", InputPhase::Move, 0.4);
    wait("the next move fails", || {
        td.volume.sets.lock().last().map(|set| set.2) == Some(0.4)
    });

    td.volume_on
        .store(false, std::sync::atomic::Ordering::SeqCst);
    td.drag("volume", &[0.5, 0.6, 0.7]);
    std::thread::sleep(Duration::from_millis(400));
    let values: Vec<f64> = td.volume.sets.lock().iter().map(|s| s.2).collect();
    assert_eq!(values, [0.3, 0.4], "the cancelled drag was still acted on");
    assert_eq!(td.log(), []);

    *td.volume.set_result.lock() = None;
    td.volume_on
        .store(true, std::sync::atomic::Ordering::SeqCst);
    td.drag("volume", &[0.2, 0.25]);
    wait("a later gesture is set", || td.last_set() == Some(0.25));
    wait("it logs", || !td.log().is_empty());
    assert_eq!(td.log(), [entry("volume", "ok", GESTURE_DONE)]);
}

#[test]
fn a_tap_on_a_part_acts_on_the_widget_account_with_the_part_unchanged() {
    let mut layout = media();
    layout["pages"][0]["tiles"][2]["action"]["account"] = json!("chosen");
    let td = running(layout, true);
    td.deck
        .emit_press_part("media", "playing", "widget", "next");
    wait("first part completes", || td.log().len() == 1);
    std::thread::sleep(Duration::from_millis(20));
    td.deck.emit_press_part("media", "volume", "widget", "mute");
    wait("both parts run", || td.log().len() == 2);
    assert_eq!(
        *td.media.acts.lock(),
        [("chosen".to_owned(), "next".to_owned(), ACT_TIMEOUT)]
    );
    assert_eq!(
        *td.volume.acts.lock(),
        [(String::new(), "mute".to_owned(), ACT_TIMEOUT)]
    );
    assert_eq!(
        td.log(),
        [
            entry("playing", "ok", "Next track."),
            entry("volume", "ok", "Mute changed.")
        ]
    );
}

#[test]
fn a_failed_control_logs_its_own_words_once() {
    let td = running(media(), true);
    *td.media.act_result.lock() = Some(ControlError::Unavailable("Nothing is playing.".into()));
    *td.volume.set_result.lock() = Some(ControlError::Unavailable("No playback device.".into()));
    td.deck
        .emit_press_part("media", "playing", "widget", "play_pause");
    wait("the tap logs", || td.log().len() == 1);
    td.drag("volume", &[0.3, 0.4, 0.5]);
    wait("the gesture logs", || td.log().len() == 2);
    *td.media.act_result.lock() = Some(ControlError::UnknownAccount);
    td.deck
        .emit_press_part("media", "playing", "widget", "next");
    wait("the refused tap logs", || td.log().len() == 3);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        td.log(),
        [
            entry("playing", "failed", "Nothing is playing."),
            entry("volume", "failed", "No playback device."),
            entry("playing", "failed", UNAVAILABLE),
        ]
    );
}

#[test]
fn hostile_input_never_reaches_a_control_its_tile_does_not_bind() {
    let td = running(media(), true);
    for tile in ["nope", "level", "playing"] {
        td.drag(tile, &[0.9, 0.95]);
    }
    td.deck
        .emit_input("media", "volume", "value", InputPhase::Move, 7.5);
    wait("the upper bound is clamped", || td.last_set() == Some(1.0));
    td.deck
        .emit_input("media", "volume", "value", InputPhase::End, -3.0);
    wait("the clamped gesture ends", || td.last_set() == Some(0.0));
    wait("the clamped gesture logs", || td.log().len() == 1);
    let send = |event: Value| td.deck.emit_value(&event);
    let input = |seq: u32, field: &str, value: Value| {
        let mut event = json!({"m": "v2.input", "page": "media", "tile": "volume", "stream": "value",
                "phase": "end", "value": 0.5, "seq": 1_000_000 + seq, "t": 1});
        event[field] = value;
        event
    };
    send(input(1, "stream", json!("muted")));
    send(input(2, "page", json!("other")));
    send(input(3, "value", json!("0.5")));
    send(input(4, "value", Value::Null));
    send(input(5, "tile", json!("x".repeat(900))));
    for (seq, part, tile, entries) in [
        (1, "mute", "level", 2),
        (2, "next", "volume", 3),
        (3, "launch", "playing", 4),
        (4, "PLAY", "playing", 4),
        (5, "next", "nope", 5),
    ] {
        send(
            json!({"m": "press", "page": "media", "tile": tile, "kind": "widget", "part": part,
                "seq": 1_000_000 + seq, "t": 1}),
        );
        wait("the part logs before the next tap", || {
            td.log().len() == entries
        });
        std::thread::sleep(Duration::from_millis(30));
    }
    wait("the parts log", || td.log().len() == 5);
    std::thread::sleep(Duration::from_millis(200));
    let values: Vec<f64> = td.volume.sets.lock().iter().map(|s| s.2).collect();
    assert_eq!(values, [1.0, 0.0]);
    assert!(td.volume.acts.lock().is_empty());
    assert!(td.media.acts.lock().is_empty());
    assert!(td.media.sets.lock().is_empty());
    let unavailable = |tile: &str| entry(tile, "failed", UNAVAILABLE);
    assert_eq!(
        td.log(),
        [
            entry("volume", "ok", GESTURE_DONE),
            unavailable("level"),
            unavailable("volume"),
            unavailable("playing"),
            unavailable("nope"),
        ]
    );
}

#[test]
fn a_flood_of_drags_and_taps_stays_bounded_and_the_next_gesture_still_works() {
    for force_overflow in [false, true] {
        let td = running(media(), true);
        let paused = force_overflow.then(|| td.devices.pause_key_reports());
        td.deck.emit_key(&json!({"type":"kbd","mods":0,"keys":[]}));
        let started = Instant::now();
        for i in 0..5_000 {
            let value = f64::from(i % 100) / 100.0;
            td.deck
                .emit_input("media", "volume", "value", InputPhase::Move, value);
        }
        for _ in 0..200 {
            td.deck
                .emit_press_part("media", "playing", "widget", "next");
        }
        Links::new(td.devices.clone())
            .with(DEVICE, |session| {
                session.request(&td_core::Request::Ping { n: None }, Duration::from_secs(5))
            })
            .unwrap()
            .unwrap();
        drop(paused);
        let flooded = started.elapsed();
        std::thread::sleep(Duration::from_millis(300));
        let sets = td.volume.sets.lock().len();
        assert!(sets < 5_000 / 4, "{sets} sets for 5000 moves");
        let acts = td.media.acts.lock().len() as u64;
        assert!(
            acts <= 10 * (flooded.as_secs() + 2),
            "{acts} acts in {flooded:?}"
        );
        assert!(td.log().len() as u64 <= acts);

        std::thread::sleep(Duration::from_secs(1));
        td.drag("volume", &[0.25, 0.33]);
        wait("a later gesture is set", || td.last_set() == Some(0.33));
    }
}

#[test]
fn before_the_router_reads_the_layout_touches_fail_closed() {
    let td = running(media(), false);
    td.drag("volume", &[0.4, 0.5]);
    wait("the drag fails", || td.log().len() == 1);
    td.deck
        .emit_press_part("media", "playing", "widget", "previous");
    wait("the tap fails", || td.log().len() == 2);
    assert!(td.media.acts.lock().is_empty());
    assert_eq!(
        td.log(),
        [
            entry("volume", "failed", UNAVAILABLE),
            entry("playing", "failed", UNAVAILABLE)
        ]
    );
    assert!(td.volume.sets.lock().is_empty());
}
#[test]
fn an_unheld_layout_cannot_choose_a_control_account() {
    let mut layout = media();
    layout["pages"][0]["tiles"][2]["action"]["account"] = json!("forged");
    let td = running(layout, false);
    td.deck
        .emit_press_part("media", "playing", "widget", "next");
    wait("the refused tap logs", || !td.log().is_empty());
    assert!(td.media.acts.lock().is_empty());
    assert_eq!(td.log(), [entry("playing", "failed", UNAVAILABLE)]);
}

#[test]
fn a_device_read_layout_without_a_pushed_copy_never_reaches_act_or_set() {
    let td = running_with_authority(media(), true, false);
    let id = DeviceId::parse(DEVICE).unwrap();
    wait("the read Layout has no pushed copy", || {
        matches!(
            td.devices.get(&id).unwrap().control_layout,
            crate::devices::ControlLayout::NoPushedCopy {
                has_controls: true,
                ..
            }
        )
    });
    td.deck
        .emit_press_part("media", "playing", "widget", "next");
    td.drag("volume", &[0.4, 0.5]);
    wait("both touches finish", || td.log().len() == 2);
    assert!(td.media.acts.lock().is_empty());
    assert!(td.volume.sets.lock().is_empty());
    assert_eq!(
        td.log(),
        [
            entry("playing", "failed", UNAVAILABLE),
            entry("volume", "failed", UNAVAILABLE)
        ]
    );
}

#[test]
fn a_non_settable_bar_never_reaches_set() {
    let mut layout = media();
    layout["pages"][0]["tiles"][0]["action"]["field"] = json!("readonly");
    let td = running(layout, true);
    td.drag("volume", &[0.4, 0.5]);
    wait("the refusal logs", || !td.log().is_empty());
    assert!(td.volume.sets.lock().is_empty());
    assert_eq!(td.log(), [entry("volume", "failed", UNAVAILABLE)]);
}

#[test]
fn a_hung_control_and_part_flood_do_not_block_ordinary_taps() {
    let td = running(media(), true);
    *td.media.act_delay.lock() = Duration::from_secs(5);
    td.deck
        .emit_press_part("media", "playing", "widget", "next");
    wait("the control starts", || !td.media.acts.lock().is_empty());
    for _ in 0..50 {
        td.deck
            .emit_press_part("media", "playing", "widget", "next");
    }
    td.deck.emit_press("media", "nope", "none");
    let deadline = Instant::now() + Duration::from_secs(1);
    while !td.log().iter().any(|entry| entry.0 == "nope") {
        assert!(
            Instant::now() < deadline,
            "ordinary tap was blocked or dropped"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(td.media.acts.lock().len(), 1);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !td
        .log()
        .iter()
        .any(|entry| entry.3 == "The player didn't respond.")
    {
        assert!(
            Instant::now() < deadline,
            "Companion deadline was not enforced"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    td.deck
        .emit_press_part("media", "playing", "widget", "next");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(td.media.acts.lock().len(), 1);
}
#[test]
fn a_cached_forged_layout_cannot_replace_the_companions_binding() {
    let td = running(media(), true);
    let mut forged = media();
    forged["pages"][0]["tiles"][2]["action"]["account"] = json!("forged");
    let layout = td_core::Layout::parse(&serde_json::to_vec(&forged).unwrap()).unwrap();
    let links = Links::new(td.devices.clone());
    links
        .with(DEVICE, |session| {
            td_link::xfer::put_layout(session, layout.orientation, &layout)
        })
        .unwrap()
        .unwrap();
    td.authority.replace(DEVICE, || Ok::<_, ()>(None)).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    td.deck
        .emit_press_part("media", "playing", "widget", "next");
    td.drag("volume", &[0.4, 0.5]);
    wait("forged tap rejected", || td.log().len() == 2);
    assert!(td.media.acts.lock().is_empty());
    assert!(td.volume.sets.lock().is_empty());
    assert_eq!(
        td.log(),
        [
            entry("playing", "failed", UNAVAILABLE),
            entry("volume", "failed", UNAVAILABLE)
        ]
    );
}

#[test]
fn a_burst_within_queue_capacity_keeps_its_final_end_and_paces_control_sets() {
    let td = running(media(), true);
    for _ in 0..200 {
        td.deck
            .emit_input("media", "volume", "value", InputPhase::Move, 0.2);
    }
    td.deck
        .emit_input("media", "volume", "value", InputPhase::End, 0.9);
    wait("final end reaches the control", || {
        td.last_set() == Some(0.9)
    });
    wait("end logs", || td.log().len() == 1);
    let times = td.volume.set_times.lock();
    for pair in times.windows(2) {
        assert!(pair[1].duration_since(pair[0]) >= Duration::from_millis(15));
    }
}

#[test]
fn a_cancelled_gestures_error_does_not_reach_the_next_gesture() {
    let td = running(media(), true);
    *td.volume.set_result.lock() = Some(ControlError::Unavailable("old failure".into()));
    td.deck
        .emit_input("media", "volume", "value", InputPhase::Move, 0.2);
    wait("first move fails", || td.last_set() == Some(0.2));
    td.authority.replace(DEVICE, || Ok::<_, ()>(None)).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    *td.volume.set_result.lock() = None;
    td.drag("volume", &[0.6, 0.7]);
    wait("new gesture completes", || !td.log().is_empty());
    assert_eq!(td.log(), [entry("volume", "ok", GESTURE_DONE)]);
}

#[test]
fn an_external_layout_change_without_samples_updates_the_status() {
    use crate::devices::ControlLayout;
    let td = running(media(), true);
    let id = DeviceId::parse(DEVICE).unwrap();
    let status = || td.devices.get(&id).unwrap().control_layout;
    wait("initial matches", || {
        matches!(status(), ControlLayout::Matches { .. })
    });
    let mut changed = media();
    changed["pages"][0]["tiles"][2]["action"]["account"] = json!("other-account");
    let layout = td_core::Layout::parse(&serde_json::to_vec(&changed).unwrap()).unwrap();
    Links::new(td.devices.clone())
        .with(DEVICE, |session| {
            td_link::xfer::put_layout(session, layout.orientation, &layout)
        })
        .unwrap()
        .unwrap();
    wait("the changed Layout is reported", || {
        matches!(status(), ControlLayout::Differs { .. })
    });
}
