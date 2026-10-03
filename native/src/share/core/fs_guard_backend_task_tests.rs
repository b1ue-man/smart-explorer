use super::*;
use std::{io::Cursor, sync::Mutex};
use crate::share::export_config::ExportAccess;

#[derive(Default)]
struct Provider { calls: Mutex<Vec<String>> }
impl Provider {
    fn touch(&self, name: &str) -> io::Result<()> { self.calls.lock().unwrap().push(name.into()); Ok(()) }
}
impl Backend for Provider {
    fn scheme(&self) -> Scheme { Scheme::GDrive }
    fn root_display(&self) -> String { "/encoded%2520root".into() }
    fn extensions(&self) -> Option<&dyn BackendExtensions> { Some(self) }
    fn list_dir(&self, _: &str) -> VfsResult<Vec<VfsMeta>> {
        Ok(vec![VfsMeta { name: ".se-versions".into(), ..Default::default() },
            VfsMeta { name: ".held.se-recycle-0123456789abcdef".into(), ..Default::default() },
            VfsMeta { name: ".ordinary.smart-explorer-abcd.part".into(), ..Default::default() }])
    }
    fn stat(&self, _: &str) -> VfsResult<VfsMeta> { Ok(VfsMeta::default()) }
    fn open_read(&self, _: &str) -> VfsResult<Box<dyn Read + Send>> { self.touch("read")?; Ok(Box::new(Cursor::new(vec![7]))) }
    fn open_write(&self, _: &str) -> VfsResult<Box<dyn Write + Send>> { self.touch("write")?; Ok(Box::new(Cursor::new(Vec::<u8>::new()))) }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> { self.open_write(path) }
    fn rename(&self, _: &str, _: &str) -> VfsResult<()> { self.touch("rename") }
    fn remove_file(&self, _: &str) -> VfsResult<()> { self.touch("remove_file") }
    fn remove_dir(&self, _: &str) -> VfsResult<()> { self.touch("remove_dir") }
    fn mkdir_all(&self, _: &str) -> VfsResult<()> { self.touch("mkdir") }
}
impl BackendExtensions for Provider {
    fn sync_child_path(&self, parent: &str, name: &str) -> VfsResult<String> {
        Ok(format!("{}/{}", parent.trim_end_matches('/'), name.replace('%', "%25")))
    }
    fn previous_state_identities(&self) -> VfsResult<Vec<String>> { Ok(vec!["drive:proven-old-account".into()]) }
    fn replace_staged_reversible(&self, _: &str, _: &str, _: &str) -> VfsResult<bool> { self.touch("reversible")?; Ok(true) }
}

#[test]
fn review_task_host_read_only_backend_denies_write_commit_and_recycle_before_provider() {
    let provider = Arc::new(Provider::default());
    let guarded = GuardedBackend::new(provider.clone(), TargetPolicy::new(ExportAccess::ReadOnly, false, false), None);
    assert_eq!(guarded.open_write("/root/a").err().unwrap().kind(), io::ErrorKind::ReadOnlyFilesystem);
    assert_eq!(guarded.open_write_new("/root/a").err().unwrap().kind(), io::ErrorKind::ReadOnlyFilesystem);
    assert!(guarded.rename("/root/a", "/root/b").is_err());
    assert!(guarded.promote_staged("/root/a", "/root/b").is_err());
    assert!(guarded.remove_file("/root/a").is_err());
    assert!(guarded.mkdir_all("/root/a").is_err());
    assert!(guarded.finish_stage("/root/a", vfs::StageFinish::default()).is_err());
    assert!(guarded.recycle("/root/a", &vfs::RecycleExpectation { size: 7, sha256: None }).is_err());
    assert!(guarded.replace_staged_reversible("/root/a", "/root/b", "/root/.se-replace-0123456789abcdef").is_err());
    assert!(provider.calls.lock().unwrap().is_empty());
}

#[test]
fn review_task_host_guard_preserves_literal_provider_hook_and_safe_identity_aliases() {
    let provider = Arc::new(Provider::default());
    let guarded = GuardedBackend::new(provider, TargetPolicy::new(ExportAccess::ReadWrite, false, false), None);
    assert_eq!(vfs::sync_child_path(&guarded, "/stored%2520parent", "%61ux.c").unwrap(), "/stored%2520parent/%2561ux.c");
    assert_eq!(vfs::sync_child_path(&guarded, "/stored%2520parent", "aux.c").unwrap(), "/stored%2520parent/aux.c");
    assert_eq!(vfs::previous_state_identities(&guarded).unwrap(), vec!["drive:proven-old-account"]);
    assert!(guarded.replace_staged_reversible("/root/a", "/root/b", "/root/.se-replace-0123456789abcdef").unwrap());
    assert!(guarded.uncached_backend().is_none());
    let entries = guarded.list_dir("/root").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, ".ordinary.smart-explorer-abcd.part");
    let mut reader = guarded.open_read("/root/a").unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, vec![7]);
}
