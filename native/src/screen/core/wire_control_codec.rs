//! Tagged screen control encoding and decoding; domain types stay in the parent.
use std::io;

use super::super::input_model::{decode_batch, encode_batch};
use super::super::wire::{
    from_json, invalid, to_json, CONTROL_REQUEST, CONTROL_STATE, ERROR, FRAME_ACK, ICON_REPLY,
    ICON_REQUEST, INPUT, KEYFRAME_REQUEST, LIST_WINDOWS, PING, PONG, PRESENTED, PREVIEW_ACK,
    SET_MODE, SNAPSHOT, STATE, STATS, VISIBILITY, VISIBILITY_ACK, WINDOWS,
};
use super::{
    decode_u64s, encode_u64s, FrameAck, HostFrame, IconRequest, IconResponse, Ping, Pong, SetMode,
    ViewerFrame, ViewerVisibility, WindowList,
};

pub(crate) fn encode_viewer_frame(frame: &ViewerFrame) -> io::Result<(u8, Vec<u8>)> {
    Ok(match frame {
        ViewerFrame::ListFamilies => (super::super::wire::LIST_FAMILIES, b"{}".to_vec()),
        ViewerFrame::FamilySnapshot => {
            (super::super::wire::FAMILY_SNAPSHOT_REQUEST, b"{}".to_vec())
        }
        ViewerFrame::FamilyOperation(request) => {
            request.validate()?;
            (super::super::wire::FAMILY_OPERATION, to_json(request)?)
        }
        ViewerFrame::FamilyInput(request) => {
            request.validate()?;
            (super::super::wire::FAMILY_INPUT, to_json(request)?)
        }
        ViewerFrame::Presented { gop_id, frame_id } => {
            if *gop_id == 0 || *frame_id == 0 {
                return Err(invalid("presented frame needs nonzero identities"));
            }
            (PRESENTED, encode_u64s(&[*gop_id, *frame_id]))
        }
        ViewerFrame::Visibility(report) => {
            report.validate()?;
            (VISIBILITY, to_json(report)?)
        }
        ViewerFrame::SetMode(set_mode) => {
            set_mode.validate()?;
            (SET_MODE, to_json(set_mode)?)
        }
        ViewerFrame::Input(events) => (INPUT, encode_batch(events)?),
        ViewerFrame::FrameAck(ack) => (FRAME_ACK, ack.encode()),
        ViewerFrame::KeyframeRequest(request) => (KEYFRAME_REQUEST, to_json(request)?),
        ViewerFrame::Ping(ping) => (PING, encode_u64s(&[ping.sequence, ping.sent_us])),
        ViewerFrame::ListWindows => (LIST_WINDOWS, b"{}".to_vec()),
        ViewerFrame::IconRequest(request) => {
            request.validate()?;
            (ICON_REQUEST, to_json(request)?)
        }
        ViewerFrame::Snapshot(request) => (SNAPSHOT, to_json(request)?),
        ViewerFrame::ControlRequest(request) => (CONTROL_REQUEST, to_json(request)?),
        ViewerFrame::PreviewAck(ack) => (PREVIEW_ACK, ack.encode()),
    })
}

pub(crate) fn decode_viewer_frame(tag: u8, payload: &[u8]) -> io::Result<ViewerFrame> {
    if payload.len() > super::super::wire::tag_limit(tag) {
        return Err(invalid("viewer frame exceeds its tag budget"));
    }
    Ok(match tag {
        super::super::wire::LIST_FAMILIES => {
            from_json::<serde_json::Map<String, serde_json::Value>>(payload)?;
            ViewerFrame::ListFamilies
        }
        super::super::wire::FAMILY_SNAPSHOT_REQUEST => {
            from_json::<serde_json::Map<String, serde_json::Value>>(payload)?;
            ViewerFrame::FamilySnapshot
        }
        super::super::wire::FAMILY_OPERATION => {
            let request: super::super::family_protocol::FamilyOperation = from_json(payload)?;
            request.validate()?;
            ViewerFrame::FamilyOperation(request)
        }
        super::super::wire::FAMILY_INPUT => {
            let request: super::super::family_protocol::FamilyInput = from_json(payload)?;
            request.validate()?;
            ViewerFrame::FamilyInput(request)
        }
        PRESENTED => {
            let [gop_id, frame_id] = decode_u64s::<2>(payload)?;
            if gop_id == 0 || frame_id == 0 {
                return Err(invalid("presented frame needs nonzero identities"));
            }
            ViewerFrame::Presented { gop_id, frame_id }
        }
        VISIBILITY => {
            let report: ViewerVisibility = from_json(payload)?;
            report.validate()?;
            ViewerFrame::Visibility(report)
        }
        SET_MODE => {
            let set_mode: SetMode = from_json(payload)?;
            set_mode.validate()?;
            ViewerFrame::SetMode(set_mode)
        }
        INPUT => ViewerFrame::Input(decode_batch(payload)?),
        FRAME_ACK => ViewerFrame::FrameAck(FrameAck::decode(payload)?),
        KEYFRAME_REQUEST => ViewerFrame::KeyframeRequest(from_json(payload)?),
        PING => {
            let [sequence, sent_us] = decode_u64s::<2>(payload)?;
            ViewerFrame::Ping(Ping { sequence, sent_us })
        }
        LIST_WINDOWS => {
            from_json::<serde_json::Map<String, serde_json::Value>>(payload)?;
            ViewerFrame::ListWindows
        }
        SNAPSHOT => ViewerFrame::Snapshot(from_json(payload)?),
        ICON_REQUEST => {
            let request: IconRequest = from_json(payload)?;
            request.validate()?;
            ViewerFrame::IconRequest(request)
        }
        CONTROL_REQUEST => ViewerFrame::ControlRequest(from_json(payload)?),
        PREVIEW_ACK => ViewerFrame::PreviewAck(FrameAck::decode(payload)?),
        _ => return Err(invalid("unknown screen frame from the viewer")),
    })
}

pub(crate) fn encode_host_frame(frame: &HostFrame) -> io::Result<(u8, Vec<u8>)> {
    Ok(match frame {
        HostFrame::Families(list) => {
            list.validate()?;
            (super::super::wire::FAMILIES, to_json(list)?)
        }
        HostFrame::FamilySnapshot(snapshot) => {
            (super::super::wire::FAMILY_SNAPSHOT, snapshot.encode()?)
        }
        HostFrame::FamilyOperation(result) => {
            result.validate()?;
            (
                super::super::wire::FAMILY_OPERATION_RESULT,
                to_json(result)?,
            )
        }
        HostFrame::FamilyInput(result) => {
            (super::super::wire::FAMILY_INPUT_RESULT, to_json(result)?)
        }
        HostFrame::VisibilityAck(ack) => (VISIBILITY_ACK, to_json(ack)?),
        HostFrame::State(report) => (STATE, to_json(report)?),
        HostFrame::Pong(pong) => (
            PONG,
            encode_u64s(&[
                pong.sequence,
                pong.ping_sent_us,
                pong.received_us,
                pong.sent_us,
            ]),
        ),
        HostFrame::Windows(list) => {
            list.validate()?;
            (WINDOWS, to_json(list)?)
        }
        HostFrame::Stats(stats) => (STATS, to_json(stats)?),
        HostFrame::IconReply(reply) => {
            reply.validate()?;
            (ICON_REPLY, to_json(reply)?)
        }
        HostFrame::ControlState(state) => (CONTROL_STATE, to_json(state)?),
        HostFrame::Error(error) => (ERROR, to_json(error)?),
    })
}

pub(crate) fn decode_host_frame(tag: u8, payload: &[u8]) -> io::Result<HostFrame> {
    if payload.len() > super::super::wire::tag_limit(tag) {
        return Err(invalid("host frame exceeds its tag budget"));
    }
    Ok(match tag {
        super::super::wire::FAMILIES => {
            let list: super::super::family_protocol::FamilyListing = from_json(payload)?;
            list.validate()?;
            HostFrame::Families(list)
        }
        super::super::wire::FAMILY_SNAPSHOT => {
            HostFrame::FamilySnapshot(crate::remote_window::FamilySnapshot::decode(payload)?)
        }
        super::super::wire::FAMILY_OPERATION_RESULT => {
            let result: super::super::family_protocol::FamilyOperationResult = from_json(payload)?;
            result.validate()?;
            HostFrame::FamilyOperation(result)
        }
        super::super::wire::FAMILY_INPUT_RESULT => HostFrame::FamilyInput(from_json(payload)?),
        VISIBILITY_ACK => HostFrame::VisibilityAck(from_json(payload)?),
        STATE => HostFrame::State(from_json(payload)?),
        PONG => {
            let [sequence, ping_sent_us, received_us, sent_us] = decode_u64s::<4>(payload)?;
            HostFrame::Pong(Pong {
                sequence,
                ping_sent_us,
                received_us,
                sent_us,
            })
        }
        WINDOWS => {
            let list: WindowList = from_json(payload)?;
            list.validate()?;
            HostFrame::Windows(list)
        }
        STATS => HostFrame::Stats(from_json(payload)?),
        ICON_REPLY => {
            let mut reply: IconResponse = from_json(payload)?;
            reply.request.validate()?;
            if reply.content.as_ref().is_some_and(|content| {
                content
                    .bytes_for(&reply.request.icon)
                    .and_then(|bytes| super::super::app_icon::decode_png(&bytes))
                    .is_err()
            }) {
                reply.content = None;
            }
            HostFrame::IconReply(reply)
        }
        CONTROL_STATE => HostFrame::ControlState(from_json(payload)?),
        ERROR => HostFrame::Error(from_json(payload)?),
        _ => return Err(invalid("unknown screen frame from the host")),
    })
}
