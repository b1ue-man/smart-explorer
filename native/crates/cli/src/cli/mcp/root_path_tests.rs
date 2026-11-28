//! Placing targets under the working root (plan 2026-11-27 section C).
use super::{
    is_relative, join_spec, namespace_root, normalize_relative, peer_scope, place, Place, RootKind,
    WorkRoot,
};
use std::sync::Arc;

fn root(spec: &str, kind: RootKind) -> Option<Arc<WorkRoot>> {
    Some(Arc::new(WorkRoot {
        spec: spec.to_string(),
        kind,
        path: String::new(),
    }))
}

fn under(spec: &str, root_spec: &str, kind: RootKind) -> Result<String, String> {
    match place(spec, root(root_spec, kind))? {
        Place::Under(_, rel) => Ok(rel),
        Place::Absolute(spec) => Err(format!("absolute: {spec}")),
    }
}

#[test]
fn mcp_root_task_relative_targets_stay_below_the_working_root() {
    let peer = "share://direct/abc/Docs/proj";
    // Relative paths, `.` and the empty target name the root or below it.
    assert_eq!(
        under("src/main.rs", peer, RootKind::Peer).unwrap(),
        "src/main.rs"
    );
    assert_eq!(
        under("./src//lib.rs", peer, RootKind::Peer).unwrap(),
        "src/lib.rs"
    );
    assert_eq!(under(".", peer, RootKind::Peer).unwrap(), "");
    assert_eq!(under("", peer, RootKind::Peer).unwrap(), "");
    assert_eq!(under("  ", peer, RootKind::Peer).unwrap(), "");
    assert_eq!(
        under("src\\bin\\se.rs", peer, RootKind::Peer).unwrap(),
        "src/bin/se.rs"
    );
    // `..` climbs inside the root, never above it.
    assert_eq!(
        under("src/../Cargo.toml", peer, RootKind::Peer).unwrap(),
        "Cargo.toml"
    );
    let above = under("..", peer, RootKind::Peer).unwrap_err();
    assert!(above.contains("leaves the working root"), "{above}");
    assert!(above.contains(peer), "{above}");
    assert!(under("src/../../x", peer, RootKind::Peer).is_err());
    assert!(normalize_relative("a\0b", peer).is_err());

    // The root's own spelling and paths below it reuse the root.
    assert_eq!(under(peer, peer, RootKind::Peer).unwrap(), "");
    assert_eq!(
        under("share://direct/abc/Docs/proj/a.txt", peer, RootKind::Peer).unwrap(),
        "a.txt"
    );
    // Absolute targets elsewhere stay absolute, also one that climbs out.
    assert!(under("share://direct/abc/Docs/projX", peer, RootKind::Peer).is_err());
    assert!(under(
        "share://direct/abc/Docs/proj/../other",
        peer,
        RootKind::Peer
    )
    .is_err());
    assert!(under("@prod:/srv", peer, RootKind::Peer).is_err());
    assert!(under("gdrive:///Docs", peer, RootKind::Peer).is_err());
    assert!(under("//server/share/x", peer, RootKind::Peer).is_err());
    #[cfg(not(windows))]
    assert!(under("/tmp/a", peer, RootKind::Peer).is_err());
    #[cfg(windows)]
    assert!(under("C:\\tmp\\a", peer, RootKind::Peer).is_err());

    // Roots ending in a separator or a scheme root.
    assert_eq!(
        under("gdrive:///Docs/a", "gdrive:///", RootKind::Drive).unwrap(),
        "Docs/a"
    );
    assert_eq!(
        under("@prod:/srv/x", "@prod:/", RootKind::Smb).unwrap(),
        "srv/x"
    );
    assert_eq!(
        under("@prod:/srv/x", "@prod:/srv/", RootKind::Smb).unwrap(),
        "x"
    );
    #[cfg(not(windows))]
    assert_eq!(
        under("/etc/hosts", "/", RootKind::Local).unwrap(),
        "etc/hosts"
    );

    // Without a root every target stays as given (relative ones are refused
    // later, as before).
    assert!(matches!(place("src", None).unwrap(), Place::Absolute(spec) if spec == "src"));
    assert!(is_relative("src/main.rs"));
    assert!(is_relative(".."));
    assert!(!is_relative("@prod:/srv"));
    assert!(!is_relative("share://direct/a/b"));
    assert!(!is_relative("\\\\server\\share"));

    assert_eq!(join_spec(peer, ""), peer);
    assert_eq!(join_spec(peer, "a/b"), "share://direct/abc/Docs/proj/a/b");
    assert_eq!(join_spec("gdrive:///", "Docs"), "gdrive:///Docs");
    assert_eq!(join_spec("C:\\Users\\a\\", "x"), "C:\\Users\\a\\x");
}

#[test]
fn mcp_root_task_share_mounts_and_namespace_roots() {
    assert_eq!(peer_scope("/Docs/proj/a").as_deref(), Some("/Docs"));
    assert_eq!(peer_scope("/Docs").as_deref(), Some("/Docs"));
    assert_eq!(
        peer_scope("/Verbindungen/nas/x").as_deref(),
        Some("/Verbindungen/nas")
    );
    assert_eq!(peer_scope("/Verbindungen"), None);
    assert_eq!(peer_scope("/"), None);
    assert!(namespace_root(RootKind::Peer, "/"));
    assert!(!namespace_root(RootKind::Peer, "/Docs"));
    assert!(namespace_root(RootKind::Drive, ""));
    assert!(namespace_root(RootKind::Unc, "//server/share/"));
    assert!(!namespace_root(RootKind::Unc, "//server/share/dir"));
    assert!(namespace_root(RootKind::Smb, "/"));
    assert!(!namespace_root(RootKind::Smb, "/share/dir"));
    assert!(namespace_root(RootKind::Local, "C:/"));
    assert!(!namespace_root(RootKind::Vault, "/Docs"));
    assert_eq!(RootKind::Peer.label("share://room/r/d/"), "share-room");
    assert_eq!(RootKind::Peer.label("share://direct/c/"), "share-direct");
    assert!(RootKind::Drive.shares_root_connection());
    assert!(!RootKind::Local.shares_root_connection());
    assert!(!RootKind::Peer.shares_root_connection());
}
