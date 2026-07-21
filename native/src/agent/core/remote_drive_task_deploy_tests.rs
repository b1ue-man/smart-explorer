use super::deploy::{agent_cache_path, require_remote_sha256, sha256_hex};

#[test]
fn remote_drive_task_agent_path_uses_protocol_and_complete_sha256() {
    let hash = sha256_hex(b"abc");
    assert_eq!(
        hash,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let path = agent_cache_path("/home/test/.cache/smart-explorer", &hash);
    assert_eq!(
        path,
        format!(
            "/home/test/.cache/smart-explorer/se-agent-p{}-{hash}",
            crate::agent_proto::PROTO_VERSION
        )
    );
    assert!(path.ends_with(&hash));
}

#[test]
fn remote_drive_task_agent_rejects_changed_remote_binary() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("se-agent");
    std::fs::write(&path, b"trusted agent bytes").unwrap();
    let backend = crate::vfs::LocalBackend::new(&temporary.path().to_string_lossy());
    let path = path.to_string_lossy();
    let expected = sha256_hex(b"trusted agent bytes");

    require_remote_sha256(&backend, &path, &expected).unwrap();
    std::fs::write(path.as_ref(), b"changed agent bytes").unwrap();
    let error = require_remote_sha256(&backend, &path, &expected).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}
