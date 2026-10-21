//! The SMB stage several writers fill (E13, U48-7, security answer S14): a
//! range handle writes only into the file that was created, never creates a
//! missing name, and a server without a usable file identity gets no ranges.
//! The stage lives in memory here; the last test needs a real server.
use super::segmented_stage::{
    identity_from, judge, open_with, standard_from, Deviation, Facts, Identity, StageWire,
    SMB_SEGMENTED_WRITE,
};
use super::segmented_stage_fake::{foreign, Store};
use crate::connect::sync_reliability_task_provider_fixture as fixture;
use crate::vfs::SegmentedStage;
use smb2::msg::create::CreateAction;
use std::io::{self, Write};
use std::sync::atomic::Ordering;

const STAGE: &str = "/share/dir/file.bin.se-upload-00ff00ff00ff00ff";

fn some(bytes: &[u8]) -> Option<&[u8]> {
    Some(bytes)
}

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 253) as u8).collect()
}

#[test]
fn speed_task_smb_segment_refuses_swapped_stage() {
    let store = Store::new();
    let stage = open_with(&store, STAGE, 30, true).unwrap().unwrap();
    let other = store.swap(STAGE, foreign(30));
    // The first range: refused, nothing written, the file streams at once.
    let error = stage.segment(0, 10).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported, "{error}");
    assert_eq!(
        store.object(other),
        vec![0xEE; 30],
        "not a byte in the other file"
    );
    assert_eq!(
        store.open.load(Ordering::SeqCst),
        1,
        "only the creator's handle"
    );
    assert!(store.refused(), "this connection offers no ranges now");
    // For the next file too.
    assert!(open_with(&store, "/share/dir/g.se-upload-01", 30, true)
        .unwrap()
        .is_none());
    drop(stage);

    // After a range was written, a swap ends the attempt (the retry streams).
    let store = Store::new();
    let stage = open_with(&store, STAGE, 30, true).unwrap().unwrap();
    let mut first = stage.segment(0, 10).unwrap();
    first.write_all(&data(10)).unwrap();
    first.flush().unwrap();
    let other = store.swap(STAGE, foreign(30));
    let error = stage.segment(10, 10).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted, "{error}");
    assert_eq!(store.object(other), vec![0xEE; 30]);
    assert!(store.refused());

    // Every other way the name can differ from the stage that was created.
    let created = Identity::Index {
        index: 7,
        created: 5,
    };
    let good = Facts {
        identity: Some(created),
        action: CreateAction::FileOpened,
        attributes: 0,
        end_of_file: 30,
        directory: false,
        delete_pending: false,
    };
    assert_eq!(judge(&created, 30, &good), Ok(()));
    let table = [
        (
            Facts {
                directory: true,
                ..good
            },
            Deviation::Directory,
        ),
        (
            Facts {
                attributes: 0x10,
                ..good
            },
            Deviation::Directory,
        ),
        (
            Facts {
                attributes: 0x400,
                ..good
            },
            Deviation::ReparsePoint,
        ),
        (
            Facts {
                delete_pending: true,
                ..good
            },
            Deviation::DeletePending,
        ),
        (
            Facts {
                end_of_file: 31,
                ..good
            },
            Deviation::Size,
        ),
        (
            Facts {
                identity: Some(Identity::Index {
                    index: 7,
                    created: 6,
                }),
                ..good
            },
            Deviation::Identity,
        ),
        (
            Facts {
                identity: Some(Identity::FileId {
                    volume: 1,
                    id: [7; 16],
                }),
                ..good
            },
            Deviation::Identity,
        ),
        (
            Facts {
                identity: None,
                ..good
            },
            Deviation::Identity,
        ),
        (
            Facts {
                action: CreateAction::FileCreated,
                ..good
            },
            Deviation::Created,
        ),
    ];
    for (facts, expected) in table {
        assert_eq!(judge(&created, 30, &facts), Err(expected), "{facts:?}");
    }
}

#[test]
fn speed_task_smb_segment_never_creates_missing_stage() {
    let store = Store::new();
    let stage = open_with(&store, STAGE, 30, true).unwrap().unwrap();
    store.remove(STAGE);
    let error = stage.segment(0, 10).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported, "{error}");
    assert_eq!(store.id_of(STAGE), None, "no new name stays behind");
    assert_eq!(store.created_by_open.load(Ordering::SeqCst), 0);
    drop(stage);

    // A server that created the name anyway (never after FILE_OPEN): the
    // handle is closed unwritten and the name is removed again.
    let store = Store::new();
    let stage = open_with(&store, STAGE, 30, true).unwrap().unwrap();
    *store.action.lock().unwrap() = CreateAction::FileCreated;
    let error = stage.segment(0, 10).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported, "{error}");
    assert_eq!(store.id_of(STAGE), None);
    assert_eq!(
        store.open.load(Ordering::SeqCst),
        1,
        "only the creator's handle"
    );
}

#[test]
fn speed_task_smb_segment_without_file_id_streams() {
    for (no_identity, zero_identity) in [(true, false), (false, true)] {
        let mut store = Store::new();
        store.no_identity = no_identity;
        store.zero_identity = zero_identity;
        let stage = open_with(&store, STAGE, 30, true).unwrap();
        assert!(stage.is_none(), "no identity, no ranges ({no_identity})");
        assert_eq!(store.id_of(STAGE), None, "the stage that was made is gone");
        assert_eq!(store.open.load(Ordering::SeqCst), 0, "its handle is closed");
        assert!(!store.refused(), "one server's answer is no refusal");
    }
    // Which identity a server gives: the file id when it answers that class,
    // else the index with the creation time; neither is none; 0 is unusable.
    let mut file_id = vec![0u8; 24];
    file_id[0] = 9;
    file_id[8] = 1;
    let index = 77u64.to_le_bytes();
    assert_eq!(
        identity_from(some(&file_id), some(&index), 5),
        Some(Identity::FileId {
            volume: 9,
            id: file_id[8..24].try_into().unwrap()
        })
    );
    assert_eq!(
        identity_from(None, some(&index), 5),
        Some(Identity::Index {
            index: 77,
            created: 5
        })
    );
    assert_eq!(identity_from(None, None, 5), None);
    assert_eq!(identity_from(some(&[1, 2, 3]), None, 5), None);
    assert!(!identity_from(None, some(&0u64.to_le_bytes()), 5)
        .unwrap()
        .is_usable());
    assert!(!identity_from(some(&[0u8; 24]), None, 5)
        .unwrap()
        .is_usable());
    // Switched off or refused: no stage is even created.
    let store = Store::new();
    assert!(open_with(&store, STAGE, 30, false).unwrap().is_none());
    store.refuse();
    assert!(open_with(&store, STAGE, 30, true).unwrap().is_none());
    assert_eq!(store.id_of(STAGE), None);
    let switch = std::hint::black_box(SMB_SEGMENTED_WRITE);
    assert!(switch, "the answer S14 turned the switch on");
}

#[test]
fn speed_task_smb_segment_finish_checks_identity_and_size_at_a_handle() {
    let store = Store::new();
    let content = data(30);
    let stage = open_with(&store, STAGE, 30, true).unwrap().unwrap();
    // Out of order, side by side.
    let mut last = stage.segment(20, 10).unwrap();
    let mut first = stage.segment(0, 10).unwrap();
    let mut middle = stage.segment(10, 10).unwrap();
    middle.write_all(&content[10..20]).unwrap();
    last.write_all(&content[20..30]).unwrap();
    first.write_all(&content[0..10]).unwrap();
    for writer in [&mut last, &mut first, &mut middle] {
        writer.flush().unwrap();
    }
    drop((last, first, middle));
    stage.finish().unwrap();
    assert_eq!(store.named(STAGE), Some(content));
    assert_eq!(
        store.open.load(Ordering::SeqCst),
        0,
        "every handle is closed"
    );

    // A swapped stage fails the finish, with nothing published from it.
    let store = Store::new();
    let stage = open_with(&store, STAGE, 30, true).unwrap().unwrap();
    store.swap(STAGE, foreign(30));
    assert!(stage.finish().is_err());
    assert_eq!(store.open.load(Ordering::SeqCst), 0);

    // A range behind the end of the stage is refused before a handle opens.
    let store = Store::new();
    let stage = open_with(&store, STAGE, 30, true).unwrap().unwrap();
    assert_eq!(
        stage.segment(25, 10).err().unwrap().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        stage.segment(u64::MAX, 2).err().unwrap().kind(),
        io::ErrorKind::InvalidInput
    );
    // Abort: the creator's handle closes first, then the name goes.
    stage.abort();
    assert_eq!(store.open.load(Ordering::SeqCst), 0);
    assert_eq!(store.id_of(STAGE), None);
    // What FileStandardInformation says.
    let mut standard = vec![0u8; 24];
    standard[8..16].copy_from_slice(&30u64.to_le_bytes());
    standard[20] = 1;
    assert_eq!(standard_from(&standard), Some((30, true, false)));
    assert_eq!(standard_from(&standard[..10]), None);
}

#[test]
#[ignore = "requires the suite-owned real SMB fixture (SE_SYNC_PROVIDER_MANIFEST)"]
fn speed_task_smb_segments_end_to_end() {
    const MIB: usize = 1024 * 1024;
    let providers = fixture::providers();
    let provider = providers
        .iter()
        .find(|provider| provider.name == "smb")
        .expect("the segmented SMB stage requires its real SMB fixture");
    let (backend, root) = provider.open("segments-end-to-end");
    let size = 3 * MIB + 777;
    let content: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
    let stage_path = fixture::path(&*backend, &root, "e2e.bin.se-upload-00ff00ff00ff00ff");
    let stage = backend
        .open_segmented_stage(&stage_path, size as u64)
        .unwrap()
        .expect("the server gives the stage a file identity");
    let ranges = [(0, MIB), (MIB, MIB), (2 * MIB, size - 2 * MIB)];
    let shared: &dyn SegmentedStage = &*stage;
    std::thread::scope(|scope| {
        for (offset, len) in ranges {
            let bytes = &content[offset..offset + len];
            scope.spawn(move || {
                let mut writer = shared.segment(offset as u64, len as u64).unwrap();
                for piece in bytes.chunks(100 * 1024) {
                    writer.write_all(piece).unwrap();
                }
                writer.flush().unwrap();
            });
        }
    });
    stage.finish().unwrap();
    let destination = fixture::path(&*backend, &root, "e2e.bin");
    backend
        .promote_copy_stage(&stage_path, &destination)
        .unwrap();
    assert_eq!(fixture::read(&*backend, &root, "e2e.bin"), content);
    backend.remove_file(&destination).unwrap();
}
