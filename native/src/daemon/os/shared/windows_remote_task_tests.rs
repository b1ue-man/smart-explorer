use super::windows_remote_task_rooted;
use crate::mount::MountMode;
use crate::share::CopyPastePeerFixture;
use std::io::{self, Read, Write};

#[test]
fn windows_remote_task_peer_mount_routes_collisions_and_keeps_permissions() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::with_labels(["Docs", "docs"])?;
    std::fs::create_dir(fixture.root_a.join("sub"))?;
    std::fs::create_dir(fixture.root_b.join("sub"))?;
    std::fs::write(fixture.root_a.join("sub/note.txt"), b"upper")?;
    std::fs::write(fixture.root_b.join("sub/note.txt"), b"lower")?;
    let readonly = windows_remote_task_rooted(fixture.backend.clone(), MountMode::ReadOnly)?;
    let entries = readonly.list_dir("/")?;
    assert_eq!(entries.len(), 2);
    assert_ne!(crate::mount::windows_ordinal_key(&entries[0].name),
        crate::mount::windows_ordinal_key(&entries[1].name));
    let mut contents = Vec::new();
    for entry in &entries {
        let root = format!("/{}", entry.name);
        assert_eq!(readonly.stat(&root)?.name, entry.name);
        let path = format!("{root}/sub/note.txt");
        let mut bytes = Vec::new();
        readonly.open_read(&path)?.read_to_end(&mut bytes)?;
        contents.push(bytes);
        assert_eq!(readonly.remove_file(&path).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }
    contents.sort();
    assert_eq!(contents, [b"lower".to_vec(), b"upper".to_vec()]);
    assert_eq!(readonly.stat("/DOCS").unwrap_err().kind(), io::ErrorKind::InvalidData);
    assert!(readonly.stat("/../Docs").is_err());

    // Exercise the same destination resolver separately from mount admission:
    // the synthetic container remains RO in the product's mount manager.
    let writable = windows_remote_task_rooted(fixture.backend.clone(), MountMode::ReadWrite)?;
    let upper = entries.iter().find(|entry| entry.name.starts_with("Docs ")).unwrap();
    let upper_root = format!("/{}", upper.name);
    let stage = format!("{upper_root}/fresh.se-mount-1234567890abcdef");
    let mut writer = writable.open_write_new(&stage)?;
    writer.write_all(b"new")?;
    writer.flush()?;
    drop(writer);
    writable.rename(&stage, &format!("{upper_root}/sub/new.txt"))?;
    assert_eq!(std::fs::read(fixture.root_a.join("sub/new.txt"))?, b"new");
    assert!(!fixture.root_b.join("sub/new.txt").exists());
    writable.remove_file(&format!("{upper_root}/sub/new.txt"))?;
    assert!(!fixture.root_a.join("sub/new.txt").exists());
    assert_eq!(fixture.backend.list_dir("/")?.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Docs", "docs"]);
    Ok(())
}
