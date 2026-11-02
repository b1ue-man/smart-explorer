//! DSK-M4: the drive reads of se Desktop go out in their wire form. Their
//! envelope is the one every `IpcRequest::Drive` has; DRV-M6 (U17) extends
//! this check with `DevicesStatus` and `DevicesWatch` once the variants
//! exist.
use serde_json::json;

use crate::daemon::drive_modules::ipc_protocol_drives::DriveIpcRequest;
use crate::daemon::ipc_protocol::IpcRequest;
use crate::mount::MountId;

#[test]
fn peek_requests_follow_the_drive_wire_form() {
    let request = IpcRequest::Drive {
        token: "geheim".into(),
        req: DriveIpcRequest::SetOptions {
            id: MountId::parse("drive-1").unwrap(),
            auto_remount: true,
        },
    };
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({
            "t": "drive",
            "token": "geheim",
            "req": {"op": "set_options", "id": "drive-1", "auto_remount": true}
        })
    );
}
