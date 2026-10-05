//! Both upload paths against the suite-owned strict, verified FTPS server.
use crate::connect::sync_reliability_task_provider_fixture as fixture;
use std::io::Write;

#[test]
fn sync_reliability_task_provider_ftps_both_writers_confirm_bytes_after_tls_close() {
    let providers = fixture::providers();
    let provider = providers
        .iter()
        .find(|provider| provider.name == "ftps")
        .expect("Linux C04 requires its real strict FTPS fixture");
    let child = "data-finish-writers";
    let (backend, root) = provider.open(child);
    let identity = backend.state_identity();
    for size in [0, 37, 128 * 1024] {
        let bytes = vec![0x5a; size];
        let spooled = format!("spooled-{size}.bin");
        let streamed = format!("streamed-{size}.bin");
        let path = fixture::path(&*backend, &root, &spooled);
        let mut writer = backend.open_write(&path).unwrap();
        writer.write_all(&bytes).unwrap();
        writer.flush().unwrap();
        writer.flush().unwrap();
        assert!(writer.write_all(b"late").is_err());
        drop(writer);
        assert_eq!(fixture::read(&*backend, &root, &spooled), bytes);

        let path = fixture::path(&*backend, &root, &streamed);
        let mut writer = backend
            .open_write_copy_stage_sized(&path, size as u64)
            .unwrap();
        writer.write_all(&bytes).unwrap();
        writer.flush().unwrap();
        writer.flush().unwrap();
        assert!(writer.write_all(b"late").is_err());
        drop(writer);
        assert_eq!(fixture::read(&*backend, &root, &streamed), bytes);
    }
    drop(backend);
    let (backend, reopened_root) = provider.open(child);
    assert_eq!(root, reopened_root);
    assert_eq!(backend.state_identity(), identity);
    for size in [0, 37, 128 * 1024] {
        for kind in ["spooled", "streamed"] {
            let relative = format!("{kind}-{size}.bin");
            assert_eq!(fixture::read(&*backend, &root, &relative), vec![0x5a; size]);
        }
    }
}
