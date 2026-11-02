use super::peer_root_paths_for_test;

#[test]
fn remote_drive_task_peer_root_choices_are_safe_deduplicated_and_concrete_first() {
    let roots = peer_root_paths_for_test(&[
        ("Docs", &[]),
        ("Docs", &[]),
        ("bad/root", &[]),
        ("bad\\root", &[]),
        ("bad\nroot", &[]),
        ("..", &[]),
        ("Verbindungen (2)", &[]),
        (
            "Verbindungen",
            &["Server", "Server", "../escape", "nested/path", ""],
        ),
    ]);

    assert_eq!(
        roots,
        vec![
            "/Docs".to_string(),
            "/Verbindungen (2)".to_string(),
            "/Verbindungen/Server".to_string(),
            "/".to_string(),
        ]
    );
    assert_eq!(roots.last().map(String::as_str), Some("/"));
}
