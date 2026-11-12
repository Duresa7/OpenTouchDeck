use td_core::layout::Layout;

#[test]
fn bluetooth_tiles_require_the_advertised_sink_and_round_trip() {
    let bytes = include_bytes!("../../../../schema/examples/bluetooth.json");
    let layout = Layout::parse(bytes).unwrap();
    assert!(layout.validate_sinks(&["link".into()]).is_err());
    assert!(layout.validate_sinks(&[]).is_err());
    assert!(
        layout
            .validate_sinks(&["hid".into(), "link".into()])
            .is_ok()
    );
    let encoded = serde_json::to_vec(&layout).unwrap();
    assert_eq!(Layout::parse(&encoded).unwrap(), layout);
    let bad = String::from_utf8(bytes.to_vec())
        .unwrap()
        .replace("\"hid\"", "\"unknown\"");
    assert!(Layout::parse(bad.as_bytes()).is_err());
}

#[test]
fn legacy_tiles_need_no_hid_but_bluetooth_media_must_fit_the_report_map() {
    let legacy = Layout::parse(include_bytes!("../../../../schema/examples/builtin.json")).unwrap();
    assert!(legacy.validate_sinks(&[]).is_ok());
    let text = std::str::from_utf8(include_bytes!("../../../../schema/examples/bluetooth.json")).unwrap();
    let wide = text.replace("205", "1024");
    let layout = Layout::parse(wide.as_bytes()).unwrap();
    assert!(layout.validate_sinks(&["hid".into()]).is_err());
}
