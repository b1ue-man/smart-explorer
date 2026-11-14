//! E17: announcements under lookup ids that run out after a Direct-code
//! renewal, next to the current one (`LanAnnouncement::retiring`).
use std::collections::HashMap;

use super::{
    plain_properties, sealed_properties, LanAnnouncement, LanProof, INSTANCE_PREFIX,
    SEALED_INSTANCE_PREFIX,
};

/// One retiring announcement; always sealed `v=3` plus plain `v=2`, since
/// its holders may be older versions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetiringLanAnnouncement {
    pub hashed_id: String,
    pub proof: LanProof,
}

/// Adds the records of every retiring announcement; `views` names the
/// instance (id, ports) each record is registered under.
pub(super) fn append_records(
    announcement: &LanAnnouncement,
    records: &mut Vec<(&'static str, HashMap<String, String>)>,
    views: &mut Vec<LanAnnouncement>,
) {
    for retiring in &announcement.retiring {
        let view = LanAnnouncement {
            hashed_id: retiring.hashed_id.clone(),
            retiring: Vec::new(),
            ..announcement.clone()
        };
        if let Some(sealed) = &retiring.proof.sealed {
            records.push((
                SEALED_INSTANCE_PREFIX,
                sealed_properties(&view, &retiring.proof, sealed),
            ));
            views.push(view.clone());
        }
        records.push((
            INSTANCE_PREFIX,
            plain_properties(&view, Some(&retiring.proof)),
        ));
        views.push(view);
    }
}

/// Equal records apart from the proofs (`same_proof` compares those).
pub(super) fn same_records(current: &LanAnnouncement, next: &LanAnnouncement) -> bool {
    current.hashed_id == next.hashed_id
        && current.p4 == next.p4
        && current.p6 == next.p6
        && current.uplink == next.uplink
        && current.plain_v2 == next.plain_v2
        && current.retiring.len() == next.retiring.len()
        && current
            .retiring
            .iter()
            .zip(&next.retiring)
            .all(|(left, right)| {
                left.hashed_id == right.hashed_id && same_proof(&left.proof, &right.proof)
            })
}

/// A fresh proof with the same facts keeps the registered record until it
/// is about to expire (every proof carries a new random seal nonce).
pub(super) fn same_proof(previous: &LanProof, next: &LanProof) -> bool {
    previous.epoch == next.epoch
        && previous.addresses == next.addresses
        && previous.sealed.is_some() == next.sealed.is_some()
        && previous.expires_at > super::super::core::now_secs().saturating_add(60)
}

#[cfg(test)]
#[path = "code_renewal_grenze_task_tests.rs"]
mod grenze_task_tests;
