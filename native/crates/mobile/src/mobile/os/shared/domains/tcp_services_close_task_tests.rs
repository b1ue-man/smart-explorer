//! E50 on Android: „Schließen“ of a service the device's feed does not name
//! ends with the same message as `se tcp close-service` and the desktop.
use super::close;
use serde_json::json;

#[test]
fn dienste_oeffnen_task_android_close_of_an_unknown_service_names_the_port() {
    let error = close(&json!({
        "deviceId": "dienste-android-geraet",
        "publicKey": "dienste-android-schluessel",
        "port": 8080,
    }))
    .expect_err("an unknown service is no successful close");
    assert_eq!(error.kind, "invalid");
    assert_eq!(
        error.message,
        "Port 8080 ist auf dem Gerät nicht freigegeben"
    );
}
