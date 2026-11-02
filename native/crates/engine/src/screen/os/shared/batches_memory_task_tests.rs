//! Sources only; these fixtures are invoked by the single remote task suite.
#[cfg(target_os = "linux")]
#[test]
fn batches_task_b6_linux_cgroup_ancestors_constrain_session_headroom() {
    use super::super::platform_session_memory::hierarchy_headroom;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let child = root.join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::write(root.join("memory.max"), "1073741824").unwrap();
    std::fs::write(root.join("memory.current"), "805306368").unwrap();
    std::fs::write(child.join("memory.max"), "max").unwrap();
    std::fs::write(child.join("memory.current"), "1048576").unwrap();
    assert_eq!(
        hierarchy_headroom(root, &child, true),
        Some(256 * 1024 * 1024)
    );
    std::fs::write(child.join("memory.max"), "2097152").unwrap();
    assert_eq!(hierarchy_headroom(root, &child, true), Some(1024 * 1024));
    std::fs::write(root.join("memory.current"), "unreadable").unwrap();
    assert_eq!(hierarchy_headroom(root, &child, true), Some(1024 * 1024));
    std::fs::write(root.join("memory.current"), "805306368").unwrap();
    std::fs::write(child.join("memory.current"), "unreadable").unwrap();
    assert_eq!(hierarchy_headroom(root, &child, true), None);
    std::fs::remove_dir_all(&child).unwrap();
    assert_eq!(hierarchy_headroom(root, &child, true), None);
    std::fs::create_dir(&child).unwrap();
    assert_eq!(hierarchy_headroom(root, &child, true), None);
}

#[cfg(target_os = "linux")]
#[test]
fn batches_task_b6_cgroup_namespace_root_needs_kernel_root_evidence() {
    use super::super::platform_session_memory::cgroup_headroom_at;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("cgroup");
    std::fs::create_dir(&root).unwrap();
    let member = temp.path().join("membership");
    let mounts = temp.path().join("mountinfo");
    std::fs::write(&member, "0::/\n").unwrap();
    std::fs::write(
        &mounts,
        format!("31 20 0:28 / {} rw - cgroup2 cgroup rw\n", root.display()),
    )
    .unwrap();
    std::fs::write(root.join("cgroup.controllers"), "memory cpu\n").unwrap();
    // The genuine global root has neither CFTYPE_NOT_ON_ROOT file.
    assert_eq!(cgroup_headroom_at(&member, &mounts), Some(u64::MAX));
    // A namespaced non-root can have the same '/' membership and mount root.
    std::fs::write(root.join("cgroup.events"), "populated 1\n").unwrap();
    std::fs::write(root.join("cgroup.type"), "domain\n").unwrap();
    std::fs::write(root.join("memory.max"), "max").unwrap();
    std::fs::write(root.join("memory.current"), "0").unwrap();
    assert_eq!(
        cgroup_headroom_at(&member, &mounts),
        Some(512 * 1024 * 1024 * 4)
    );
    std::fs::write(root.join("memory.max"), "104857600").unwrap();
    std::fs::write(root.join("memory.current"), "103809024").unwrap();
    assert_eq!(cgroup_headroom_at(&member, &mounts), Some(1024 * 1024));
    std::fs::remove_file(root.join("cgroup.events")).unwrap();
    std::fs::remove_file(root.join("cgroup.type")).unwrap();
    std::fs::write(root.join("memory.max"), "max").unwrap();
    // EISDIR is a read failure, never proof that the non-root file is absent.
    std::fs::create_dir(root.join("cgroup.events")).unwrap();
    assert_eq!(
        cgroup_headroom_at(&member, &mounts),
        Some(512 * 1024 * 1024 * 4)
    );
    std::fs::remove_dir(root.join("cgroup.events")).unwrap();
    std::fs::remove_file(&mounts).unwrap();
    assert_eq!(cgroup_headroom_at(&member, &mounts), None);
}
