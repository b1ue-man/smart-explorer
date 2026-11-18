//! Task tests of the fix of 2026-11-18 (host log of a Windows VM): the
//! refusal of a claim on a retired grant is recognisable as turnover.
use std::io;

use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner};
use crate::screen::core::capture_test_support;
use crate::screen::core::pixel_convert::FrameSize;

#[test]
fn shadowhost_task_retired_grant_claim_is_recognisable_turnover() {
    let grant = capture_test_support::grant(FrameSize::new(4, 2), None);
    // A live grant claims; its errors are no turnover.
    let slice = grant.claim(CaptureOwner::SourcePixels, 8).unwrap();
    drop(slice);
    let too_large = grant
        .claim(CaptureOwner::SourcePixels, u64::MAX)
        .err()
        .expect("the retired grant refuses the claim");
    assert!(!CaptureGrant::is_retired_claim(&too_large));
    assert!(!CaptureGrant::is_retired_claim(&io::Error::other(
        "retired capture grant cannot authorize new claims"
    )));
    // A retired grant refuses with the typed turnover error; kind and text
    // stay as before.
    grant.retire().unwrap();
    let refused = grant
        .claim(CaptureOwner::SourcePixels, 8)
        .err()
        .expect("the retired grant refuses the claim");
    assert!(CaptureGrant::is_retired_claim(&refused));
    assert_eq!(refused.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        refused.to_string(),
        "retired capture grant cannot authorize new claims"
    );
}
