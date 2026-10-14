//! Delete-speed task tests for SMB folder deletion without a server: the
//! share root and the server level stay with the generic walk. The shared
//! pipelined runner is tested with SFTP; the wire run (credits, link
//! deletes) needs a real SMB server.
use super::url::entry_path;

#[test]
fn delete_speed_task_smb_tree_delete_needs_an_entry_inside_a_share() {
    // `supports_remove_tree` is `entry_path(path).is_ok()`.
    assert!(entry_path("/share/ordner").is_ok());
    assert!(entry_path("/share").is_err());
    assert!(entry_path("/").is_err());
}
