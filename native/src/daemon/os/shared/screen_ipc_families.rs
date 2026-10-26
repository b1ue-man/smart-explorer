//! Authenticated, generation-bearing window selectors on the metadata channel.
use super::*;
use crate::screen::core::family_protocol::{FamilyListing, FAMILY_PROTOCOL};

pub(crate) fn screen_families(target: PeerOpenTarget, accept_within: Option<Duration>,
    pending: impl FnMut(Capability)) -> Result<FamilyListing, String> {
    families(target, None, accept_within, pending)
}

pub(crate) fn screen_families_for_node(target: PeerOpenTarget, expected_node: &str,
    accept_within: Option<Duration>, pending: impl FnMut(Capability)) -> Result<FamilyListing, String> {
    families(target, Some(expected_node), accept_within, pending)
}

fn families(target: PeerOpenTarget, expected_node: Option<&str>, accept_within: Option<Duration>,
    mut pending: impl FnMut(Capability)) -> Result<FamilyListing, String> {
    let mut session = Relayed::open_for_node(target, None, true, expected_node)?;
    let hello = accept(&mut session, accept_within, &mut pending)?;
    if hello.family_version != Some(FAMILY_PROTOCOL) {
        return Err("window_family_unsupported: Der Host bietet keine bestätigten Fensterfamilien an".into());
    }
    session.send(&ViewerFrame::ListFamilies).map_err(|error|error.to_string())?;
    let deadline = Instant::now() + READ_TIMEOUT;
    loop {
        match event_before(&mut session, deadline)? {
            StreamEvent::Host(HostFrame::Families(list)) => {
                list.validate().map_err(|error|error.to_string())?;
                return Ok(list);
            }
            StreamEvent::Host(HostFrame::Error(error)) => return Err(error_text(&error)),
            StreamEvent::End(end) => return Err(end_text(&end)),
            _ => {},
        }
    }
}
