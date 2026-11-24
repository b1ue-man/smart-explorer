//! The `HELLO_OK` a host answers with: authorization, platform, encoders,
//! input, monitors, limits, approval, frame contract and the announced
//! capabilities (round 12 rest: `input_receipts`). Split from
//! `host_server.rs`.
use super::*;

impl Start<'_> {
    pub(super) fn hello_ok(
        &self,
        metadata: &HostMetadata,
        authorization: ScreenAuthorization,
        approval: ApprovalKind,
        frame_contract: Option<FrameContract>,
    ) -> ScreenHelloOk {
        let mut ok = ScreenHelloOk {
            authorization,
            platform: metadata.platform,
            encoders: Vec::new(),
            encoders_v2: None,
            input: metadata.input.clone(),
            monitors: metadata.monitors.clone(),
            limits: metadata.limits.clone(),
            approval,
            frame_contract,
            icon_version: self.icon_version,
            family_version: self.family_version,
            virtual_display: None,
            audio: None,
            input_receipts: Some(crate::screen::core::input_receipt::INPUT_RECEIPTS_V1),
        };
        ok.offer_encoders(&metadata.encoders, self.hello);
        ok.set_virtual_display(metadata.virtual_display.clone());
        ok
    }
}
