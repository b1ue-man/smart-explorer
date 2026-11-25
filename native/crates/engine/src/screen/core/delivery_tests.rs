use super::*;

#[test]
fn delivery_task_keys_labels_and_default() {
    assert_eq!(Delivery::default(), Delivery::Realtime);
    for mode in Delivery::ALL {
        assert_eq!(Delivery::parse(mode.key()), Some(mode));
        assert!(!mode.label().is_empty() && !mode.hint().is_empty());
    }
    assert_eq!(Delivery::parse("udp"), None);
    assert_eq!(
        serde_json::to_string(&Delivery::Reliable).unwrap(),
        "\"reliable\""
    );
    assert_eq!(
        serde_json::from_str::<Delivery>("\"realtime\"").unwrap(),
        Delivery::Realtime
    );
}
