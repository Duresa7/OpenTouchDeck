//! The Device page's Firmware group end to end on a mock Device: the bundled update, an update
//! from a file with the native consent scripted, a decline, a cancel while it waits, the build
//! switch, and a copy of Open TouchDeck that carries no Firmware.
use super::*;
use td_companion::devices::FirmwareChannel;

const V1: &[u8] = include_bytes!("../../../td-companion/tests/fixtures/firmware-v1.tdfw");
const RADIO: &[u8] = include_bytes!("../../../td-link/tests/fixtures/touchdeck-test-radio.tdfw");
const DEVICE: &str = "Mock Device mock0001";

fn updates(rig: &Rig) -> usize {
    rig.deck("mock0001")
        .received()
        .iter()
        .filter(|v| v["m"] == "update.begin")
        .count()
}

fn status(rig: &Rig) -> String {
    rig.app.ui.get_device_page().firmware_status.to_string()
}

/// What a failed or stopped update says.
fn result(rig: &Rig) -> String {
    rig.app.ui.get_device_page().firmware_result.to_string()
}

/// A Device on Firmware 0.9.0, its page open.
fn on_old_firmware(rig: &Rig) {
    rig.ports
        .board("mock0001", BoardKind::Device)
        .set_version("0.9.0");
    rig.plug("mock0001", BoardKind::Device);
    rig.open_device_settings(DEVICE);
}

#[test]
fn the_bundled_update_is_one_click() {
    let rig = Rig::start_with(960.0, |handle| {
        handle.firmware().set_bundles(Some(V1), Some(RADIO));
        // Any consent dialog would wait for ever: a same-or-newer update never asks.
        handle.firmware().script(vec![]);
    });
    on_old_firmware(&rig);
    assert_eq!(status(&rig), "1.0.0 available");

    rig.press("Update");
    rig.until("the update to finish", |r| status(r) == "Updated to 1.0.0");
    assert_eq!(updates(&rig), 1);
    assert!(!rig.has("Update"), "nothing more to offer");
}

#[test]
fn an_update_from_a_file_waits_for_confirmation_on_this_pc() {
    let rig = Rig::start(960.0);
    rig.handle().firmware().script(vec![]);
    rig.plug("mock0001", BoardKind::Device);
    rig.open_device_settings(DEVICE);
    assert!(!rig.has("Update"), "the mock runs no version to compare");

    // Cancelled while the dialog waits: nothing changes.
    rig.press("Update from a file…");
    rig.until("the consent to wait", |r| {
        r.has("Waiting for confirmation on this PC")
    });
    assert_eq!(rig.app.ui.get_device_page().firmware_step, "waiting");
    assert!(
        !rig.one("Update from a file…", AccessibleRole::Button)
            .accessible_enabled()
            .unwrap_or(true),
        "one update at a time"
    );
    rig.press("Cancel the update");
    rig.until("the update to stop", |r| {
        result(r) == format!("Update stopped. {DEVICE} still runs mock.")
    });
    assert!(!rig.has("Waiting for confirmation on this PC"));
    assert_eq!(updates(&rig), 0);

    // Confirmed on this PC.
    rig.press("Update from a file…");
    rig.until("the consent to wait", |r| {
        r.has("Waiting for confirmation on this PC")
    });
    rig.handle().firmware().script(vec![ConsentAnswer::Accept]);
    rig.until("the update to finish", |r| {
        status(r) == "Updated to test-only"
    });
    assert_eq!(updates(&rig), 1);
}

#[test]
fn a_declined_update_from_a_file_changes_nothing() {
    let rig = Rig::start(960.0);
    rig.handle().firmware().script(vec![ConsentAnswer::Decline]);
    rig.plug("mock0001", BoardKind::Device);
    rig.open_device_settings(DEVICE);

    rig.press("Update from a file…");
    let declined = format!("Declined on this PC. {DEVICE} still runs mock.");
    rig.until("the decline", |r| result(r) == declined);
    assert_eq!(rig.app.ui.get_device_page().firmware_step, "failed");
    rig.one(&declined, AccessibleRole::Text);
    assert_eq!(updates(&rig), 0);
}

#[test]
fn the_build_switch_asks_in_the_window_then_on_this_pc() {
    let rig = Rig::start(960.0);
    rig.plug("mock0001", BoardKind::Device);
    rig.open_device_settings(DEVICE);
    let page = rig.app.ui.get_device_page();
    assert!(page.build_shown);
    assert_eq!(page.build_title, "Normal build");

    // Cancel in the window asks nothing on the PC.
    rig.press("Install the radio build");
    rig.one(
        &format!("Install the radio build on {DEVICE}?"),
        AccessibleRole::Groupbox,
    );
    rig.press("Cancel");
    rig.until("the question to close", |r| {
        !r.has(&format!("Install the radio build on {DEVICE}?"))
    });
    assert_eq!(updates(&rig), 0);

    rig.press("Install the radio build");
    rig.press("Install");
    rig.until("the switch to finish", |r| {
        status(r) == "Now on the radio build, test-only-radio"
    });
    let id = DeviceId::parse("mock0001").unwrap();
    let handle = rig.handle();
    rig.until("the radio build to report", |r| {
        handle
            .devices()
            .get(&id)
            .and_then(|d| d.firmware)
            .is_some_and(|f| f.channel == FirmwareChannel::Radio)
            && r.app.ui.get_device_page().build_title == "Radio build"
    });
    assert!(rig.has("Switch to the normal build"));
}

#[test]
fn without_bundled_firmware_the_page_offers_only_a_file() {
    let rig = Rig::start_with(960.0, |handle| handle.firmware().set_bundles(None, None));
    on_old_firmware(&rig);
    let page = rig.app.ui.get_device_page();
    assert!(!page.firmware_offer);
    assert!(!page.build_shown, "no other build to switch to");
    assert!(!rig.has("Update"));
    assert!(!rig.has("Install the radio build"));
    assert_eq!(
        status(&rig),
        "This copy of Open TouchDeck doesn't include Firmware. Update from a file instead."
    );
    assert!(
        rig.one("Update from a file…", AccessibleRole::Button)
            .accessible_enabled()
            .unwrap_or(false)
    );

    rig.press("Update from a file…");
    rig.until("the update to finish", |r| {
        status(r) == "Updated to test-only"
    });
}
