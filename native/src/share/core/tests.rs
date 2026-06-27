use super::core::{hmac_proof, presence_payload, random_bytes, sanitize_name, verify_hmac};
use super::fs;
use super::profiles::ShareProfiles;
use super::wire::{Ctrl, FsRequest};
use crate::vfs::{Backend, LocalBackend, Scheme, VfsMeta, VfsResult};
use std::collections::HashMap;
use std::io;
use std::sync::Mutex;

#[test]
fn room_code_uses_persistent_secret_format() {
    let code = ShareProfiles::new_room_code();
    let parts: Vec<&str> = code.split('-').collect();
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[0], "SE");
    assert_eq!(parts[1], "R3");
    assert_eq!(parts[3].len(), 64);
}

#[test]
fn presence_hmac_covers_iroh_discovery_fields() {
    let secret = random_bytes::<32>();
    let candidates = vec!["192.168.1.20:1234".to_string()];
    let payload = presence_payload(
        "direct",
        "lookup",
        "device-a",
        "pubkey",
        "node-a",
        "https://relay.example",
        &candidates,
        42,
        "nonce",
    );
    let proof = hmac_proof(&secret, &payload);
    assert!(verify_hmac(&secret, &payload, &proof));

    let changed_payload = presence_payload(
        "direct",
        "lookup",
        "device-a",
        "pubkey",
        "node-a",
        "https://relay.example",
        &["10.0.0.5:22".to_string()],
        42,
        "nonce",
    );
    assert!(!verify_hmac(&secret, &changed_payload, &proof));

    let changed_relay = presence_payload(
        "direct",
        "lookup",
        "device-a",
        "pubkey",
        "node-a",
        "https://evil.example",
        &candidates,
        42,
        "nonce",
    );
    assert!(!verify_hmac(&secret, &changed_relay, &proof));
}

#[test]
fn sanitize_strips_separators() {
    assert_eq!(sanitize_name("../e/t\\c:passwd"), "_e_t_c_passwd");
    assert_eq!(sanitize_name(""), "datei");
}

#[test]
fn ctrl_roundtrips() {
    let o = Ctrl::Fs {
        req: FsRequest::ListDir { path: "/".into() },
    };
    let j = serde_json::to_vec(&o).unwrap();
    assert!(matches!(
        serde_json::from_slice::<Ctrl>(&j).unwrap(),
        Ctrl::Fs { .. }
    ));
}

#[test]
fn recursive_delete_rejects_symlink_like_directory_child() {
    let be = RecordingBackend::new(
        [(
            "/root",
            vec![
                test_meta("escape", true, true),
                test_meta("keep.txt", false, false),
            ],
        )],
        [("/root/escape", test_meta("escape", true, true))],
    );

    assert!(fs::remove_dir_recursive(&be, "/root").is_err());
    let calls = be.calls();
    assert!(calls.contains(&"list:/root".to_string()));
    assert!(calls.contains(&"stat:/root/escape".to_string()));
    assert!(!calls.contains(&"list:/root/escape".to_string()));
    assert!(!calls.iter().any(|call| call.starts_with("dir:")));
    assert!(!calls.iter().any(|call| call.starts_with("file:")));
}

#[test]
fn recursive_delete_removes_normal_local_tree() {
    let root = temp_path("se-share-normal-recursive");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub").join("child.txt"), b"child").unwrap();
    std::fs::write(root.join("top.txt"), b"top").unwrap();

    let root_s = root.to_string_lossy().replace('\\', "/");
    let be = LocalBackend::new(&root_s);

    fs::remove_dir_recursive(&be, &root_s).unwrap();
    assert!(!root.exists());
}

#[test]
fn recursive_delete_does_not_follow_local_symlink_child_when_supported() {
    let base = temp_path("se-share-recursive-symlink");
    let root = base.join("root");
    let outside = base.join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), b"keep").unwrap();
    let link = root.join("link");

    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(&outside, &link);
    #[cfg(not(windows))]
    let linked = std::os::unix::fs::symlink(&outside, &link);

    if linked.is_ok() {
        let root_s = root.to_string_lossy().replace('\\', "/");
        let be = LocalBackend::new(&root_s);
        let _ = fs::remove_dir_recursive(&be, &root_s);
        assert!(outside.join("keep.txt").exists());
    }

    let _ = std::fs::remove_dir_all(base);
}

fn temp_path(prefix: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    path
}

fn test_meta(name: &str, is_dir: bool, is_symlink: bool) -> VfsMeta {
    VfsMeta {
        name: name.to_string(),
        is_dir,
        is_symlink,
        ..Default::default()
    }
}

struct RecordingBackend {
    entries: HashMap<String, Vec<VfsMeta>>,
    stats: HashMap<String, VfsMeta>,
    calls: Mutex<Vec<String>>,
}

impl RecordingBackend {
    fn new(
        entries: impl IntoIterator<Item = (&'static str, Vec<VfsMeta>)>,
        stats: impl IntoIterator<Item = (&'static str, VfsMeta)>,
    ) -> Self {
        RecordingBackend {
            entries: entries
                .into_iter()
                .map(|(path, entries)| (path.to_string(), entries))
                .collect(),
            stats: stats
                .into_iter()
                .map(|(path, meta)| (path.to_string(), meta))
                .collect(),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: impl Into<String>) {
        self.calls.lock().unwrap().push(call.into());
    }
}

impl Backend for RecordingBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }

    fn root_display(&self) -> String {
        "/root".to_string()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.record(format!("list:{path}"));
        Ok(self.entries.get(path).cloned().unwrap_or_default())
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.record(format!("stat:{path}"));
        self.stats
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing test stat"))
    }

    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn io::Read + Send>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn io::Write + Send>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
    }

    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.record(format!("file:{path}"));
        Ok(())
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.record(format!("dir:{path}"));
        Ok(())
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
    }
}
