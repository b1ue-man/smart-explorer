use super::*;

#[test]
fn is_newer_compares_semver() {
    assert!(is_newer("0.5.74", "0.5.73"));
    assert!(is_newer("0.6.0", "0.5.99"));
    assert!(is_newer("1.0.0", "0.9.9"));
    assert!(!is_newer("0.5.73", "0.5.73"));
    assert!(!is_newer("0.5.72", "0.5.73"));
    assert!(!is_newer("0.5.9", "0.5.10"));
}

#[test]
fn github_repo_and_release_tag_parsing() {
    assert_eq!(
        feed::github_repo("https://github.com/b1ue-man/smart-explorer"),
        Some(("b1ue-man".into(), "smart-explorer".into()))
    );
    assert_eq!(
        feed::github_repo("https://raw.githubusercontent.com/o/r/main/release-native/update-feed"),
        Some(("o".into(), "r".into()))
    );
    assert_eq!(feed::github_repo("https://example.com/feed"), None);
    assert_eq!(feed::github_repo("/local/dir"), None);
    assert_eq!(feed::tag_to_version("v0.5.63"), Some("0.5.63".into()));
    assert_eq!(feed::tag_to_version("vX"), None);
    assert_eq!(feed::tag_to_version("main"), None);
    assert_eq!(feed::tag_to_version("release/v0.5.63"), None);
}

#[test]
fn archived_versions_parse_and_sort_numerically() {
    let vd = archive::versions_dir().expect("versions dir");
    std::fs::create_dir_all(&vd).unwrap();
    let mk = ["0.3.6", "0.3.10", "0.4.0"];
    for v in mk {
        std::fs::write(vd.join(format!("Smart Explorer {}.exe", v)), b"x").unwrap();
    }
    let vers: Vec<String> = list_archived_versions()
        .into_iter()
        .map(|(v, _)| v)
        .collect();
    let idx = |s: &str| vers.iter().position(|x| x == s);
    assert!(idx("0.4.0").is_some() && idx("0.3.10").is_some() && idx("0.3.6").is_some());
    assert!(idx("0.4.0") < idx("0.3.10"));
    assert!(idx("0.3.10") < idx("0.3.6"));
    for v in mk {
        let _ = std::fs::remove_file(vd.join(format!("Smart Explorer {}.exe", v)));
    }
}

#[test]
fn github_repo_url_becomes_raw_feed() {
    assert_eq!(
        feed::normalize_http_feed("https://github.com/b1ue-man/smart-explorer"),
        "https://raw.githubusercontent.com/b1ue-man/smart-explorer/main/release-native/update-feed"
    );
    assert_eq!(
        feed::normalize_http_feed("https://github.com/b1ue-man/smart-explorer.git/"),
        "https://raw.githubusercontent.com/b1ue-man/smart-explorer/main/release-native/update-feed"
    );
    assert_eq!(
        feed::normalize_http_feed("https://github.com/o/r/tree/dev"),
        "https://raw.githubusercontent.com/o/r/dev/release-native/update-feed"
    );
    assert_eq!(
        feed::normalize_http_feed("https://example.com/feed/"),
        "https://example.com/feed"
    );
}

#[test]
fn classify_distinguishes_transports() {
    assert!(matches!(
        feed::classify_feed("https://example.com/f"),
        feed::Feed::Http(_)
    ));
    assert!(matches!(feed::classify_feed("http://host/f"), feed::Feed::Http(_)));
    assert!(matches!(
        feed::classify_feed(r"C:\Users\x\feed"),
        feed::Feed::Local(_)
    ));
    assert!(matches!(
        feed::classify_feed(r"\\server\share"),
        feed::Feed::Local(_)
    ));
}

#[test]
fn pin_roundtrip() {
    let had = pinned_version();
    archive::set_pin("0.3.6");
    assert!(is_auto_update_paused());
    assert_eq!(pinned_version().as_deref(), Some("0.3.6"));
    resume_auto_update();
    assert!(!is_auto_update_paused());
    if let Some(v) = had {
        archive::set_pin(&v);
    }
}
