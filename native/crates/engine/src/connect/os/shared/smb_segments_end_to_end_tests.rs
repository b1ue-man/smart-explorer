//! The segmented SMB stage end to end against the suite-owned SMB fixture. Moved
//! here from smb/core/segmented_stage_tests.rs: it names the connect fixture, which
//! lies above the transfer layer (plan docs/plaene/2026-11-01-idealzustand, D13).
use crate::connect::sync_reliability_task_provider_fixture as fixture;
use crate::vfs::SegmentedStage;
use std::io::Write;

#[test]
#[ignore = "requires the suite-owned real SMB fixture (SE_SYNC_PROVIDER_MANIFEST); Suite rechte-desktop-tresore-task"]
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
