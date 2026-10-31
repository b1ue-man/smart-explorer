//! `hash_file_v1` between two real Iroh nodes on loopback: the host hashes
//! one file of its export and sends exactly one root entry with the
//! SHA-256, `vfs::content_digest` returns it, folders and missing files are
//! errors, and a host without the feature yields `Ok(None)`.
use std::fs;
use std::io;
use std::sync::atomic::AtomicBool;

use super::CopyPastePeerFixture;
use crate::share::fs_response::{FsHashEntry, FsHashWalkMessage};
use crate::share::peer_stream;
use crate::share::wire::{FsHashAlgo, FsHashWalk, FsHostFeatures, FsRequest, FsResponse};
use crate::vfs::{content_digest, ContentDigest};

const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn idle() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
fn screen_metadata_runtime_task_share_file_walk_returns_one_item() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    fs::write(fixture.root_a.join("abc.txt"), b"abc")?;
    let request = FsRequest::HashWalk(FsHashWalk {
        path: "/A/abc.txt".into(),
        algo: Some(FsHashAlgo::Sha256),
        min_bytes: 0,
        single_file: true,
        meta: false,
    });
    let mut items: Vec<FsHashEntry> = Vec::new();
    let mut holes = 0usize;
    let totals = peer_stream::call(&fixture.peer, request, &idle(), |response| match response {
        FsResponse::HashWalk { message } => match message {
            FsHashWalkMessage::Progress { .. } => Ok(None),
            FsHashWalkMessage::Batch { entries, omitted } => {
                items.extend(entries);
                holes += omitted.len();
                Ok(None)
            }
            FsHashWalkMessage::Done { entries, omitted } => Ok(Some((entries, omitted))),
        },
        other => Err(io::Error::other(format!("unexpected reply {other:?}"))),
    })?;
    assert_eq!(totals, (1, 0));
    assert_eq!(holes, 0);
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0].rel, "");
    assert!(!items[0].is_dir);
    assert_eq!(items[0].size, 3);
    assert_eq!(items[0].digest.as_deref(), Some(ABC_SHA256));
    Ok(())
}

#[test]
fn screen_metadata_runtime_task_share_content_digest_comes_from_the_host() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    fs::create_dir(fixture.root_a.join("sub"))?;
    fs::write(fixture.root_a.join("sub").join("abc.txt"), b"abc")?;
    let expected = ContentDigest {
        sha256: ABC_SHA256.into(),
        size: 3,
    };
    let peer = &*fixture.peer;
    assert_eq!(
        content_digest(peer, "/A/sub/abc.txt", &idle())?,
        Some(expected.clone())
    );
    assert_eq!(
        content_digest(&*fixture.backend, "/A/sub/abc.txt", &idle())?,
        Some(expected)
    );
    assert!(content_digest(peer, "/A/sub", &idle()).is_err(), "a folder");
    assert!(
        content_digest(peer, "/A", &idle()).is_err(),
        "an export root"
    );
    assert!(
        content_digest(peer, "/A/sub/gone.txt", &idle()).is_err(),
        "missing"
    );

    // A host that does not offer `hash_file_v1` is never asked.
    if let Ok(mut slot) = fixture.peer.transfer.features.lock() {
        if let Some((_, features)) = slot.as_mut() {
            features.hash_file_v1 = false;
        }
    }
    assert_eq!(content_digest(peer, "/A/sub/abc.txt", &idle())?, None);
    Ok(())
}

#[test]
fn screen_metadata_runtime_task_share_file_walk_wire_stays_compatible() {
    let folder_walk = FsHashWalk {
        path: "/A".into(),
        algo: Some(FsHashAlgo::Md5),
        min_bytes: 0,
        single_file: false,
        meta: false,
    };
    let encoded = serde_json::to_string(&folder_walk).unwrap();
    assert!(!encoded.contains("single_file"), "{encoded}");
    let old_client: FsHashWalk = serde_json::from_str(r#"{"path":"/A","min_bytes":0}"#).unwrap();
    assert!(!old_client.single_file, "an older client walks folders");
    assert!(FsHostFeatures::host().hash_file_v1);
    assert!(FsHostFeatures::host().names().contains(&"hash_file_v1"));
    let old_host: FsHostFeatures = serde_json::from_str(r#"{"hash_walk_v1":true}"#).unwrap();
    assert!(!old_host.hash_file_v1, "an older host is never asked");
}
