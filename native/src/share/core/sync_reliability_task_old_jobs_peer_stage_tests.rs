//! The IPC writer's daemon purpose uses the same acknowledged creator proof.
use super::{Binding, StageLedger};
use crate::share::peer_writer::owned_writer;
use crate::vfs::VfsMeta;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

const STAGE: &str = "/Docs/file.se-daemon-0123456789abcdef";
const BYTES: &[u8] = b"acknowledged creator bytes";

fn binding() -> Binding {
    Binding::new("peer:Direct:c08:node".into(), Some("c08-lease".into()))
}

fn metadata() -> VfsMeta {
    VfsMeta {
        name: "file.se-daemon-0123456789abcdef".into(),
        size: BYTES.len() as u64,
        mtime_ms: 42,
        id: Some("exclusively-created-id".into()),
        ..Default::default()
    }
}

struct RecordingWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
    lose_ack: bool,
}

impl Write for RecordingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.lose_ack {
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "WriteDone acknowledgement unavailable",
            ))
        } else {
            Ok(())
        }
    }
}

#[test]
fn sync_reliability_task_old_jobs_daemon_stage_ack_keeps_exclusive_creator() {
    let ledger = StageLedger::default();
    let ticket = ledger.reserve(STAGE, binding()).unwrap();
    let creating = ledger.ready(STAGE, &binding()).err().unwrap();
    assert_eq!(creating.kind(), io::ErrorKind::PermissionDenied);
    assert!(creating.to_string().contains("state=creating"));
    ticket.opened().unwrap();
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let mut writer = owned_writer(
        Box::new(RecordingWriter {
            bytes: bytes.clone(),
            lose_ack: false,
        }),
        ticket,
    );
    writer.write_all(BYTES).unwrap();
    let writing = ledger.ready(STAGE, &binding()).err().unwrap();
    assert_eq!(writing.kind(), io::ErrorKind::PermissionDenied);
    assert!(writing.to_string().contains("state=writing"));
    writer.flush().unwrap();
    drop(writer);
    ledger.verify(STAGE, &binding(), &metadata()).unwrap();

    // Equal binding and path on a reopened backend are not a creator proof.
    let reopened = StageLedger::default();
    let foreign = reopened.ready(STAGE, &binding()).err().unwrap();
    assert_eq!(foreign.kind(), io::ErrorKind::PermissionDenied);
    assert!(foreign.to_string().contains("state=untracked"));
    assert!(foreign.to_string().contains(STAGE));
    assert!(!foreign.to_string().contains("c08-lease"));
    ledger.verify(STAGE, &binding(), &metadata()).unwrap();

    let replacement = VfsMeta {
        id: Some("foreign-created-id".into()),
        ..metadata()
    };
    assert_eq!(
        ledger
            .verify(STAGE, &binding(), &replacement)
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(ledger.ready(STAGE, &binding()).is_err());
    assert!(ledger.contains(STAGE).unwrap());
    let retained = bytes.lock().unwrap().clone();
    assert_eq!(retained.as_slice(), BYTES);
}

#[test]
fn sync_reliability_task_old_jobs_daemon_stage_lost_ack_and_rebinding_stay_unproven() {
    for lose_ack in [true, false] {
        let ledger = StageLedger::default();
        let ticket = ledger.reserve(STAGE, binding()).unwrap();
        ticket.opened().unwrap();
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let mut writer = owned_writer(
            Box::new(RecordingWriter {
                bytes: bytes.clone(),
                lose_ack,
            }),
            ticket,
        );
        writer.write_all(BYTES).unwrap();
        if lose_ack {
            assert_eq!(
                writer.flush().unwrap_err().kind(),
                io::ErrorKind::UnexpectedEof
            );
            assert_eq!(
                writer.flush().unwrap_err().kind(),
                io::ErrorKind::UnexpectedEof
            );
        } else {
            writer.flush().unwrap();
            let rebound = Binding::new(
                "peer:Direct:c08:node".into(),
                Some("replacement-lease".into()),
            );
            assert_eq!(
                ledger.ready(STAGE, &rebound).err().unwrap().kind(),
                io::ErrorKind::PermissionDenied
            );
        }
        drop(writer);
        let uncertain = ledger.ready(STAGE, &binding()).err().unwrap();
        assert_eq!(uncertain.kind(), io::ErrorKind::PermissionDenied);
        assert!(uncertain.to_string().contains("state=pending"));
        assert!(ledger.reserve(STAGE, binding()).is_err());
        assert!(ledger.contains(STAGE).unwrap());
        let retained = bytes.lock().unwrap().clone();
        assert_eq!(retained.as_slice(), BYTES);
    }
}
