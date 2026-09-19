use super::*;

#[test]
fn search_recursive_access_task_helper_admits_exact_read_protocol_only() {
    let args: Vec<OsString> = [
        HELPER_FLAG.to_string(),
        format!("{PIPE_PREFIX}{}", "a".repeat(32)),
        "42".into(),
        "b".repeat(64),
        "C:/allowed folder/".into(),
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    let request = parse(&args).unwrap().unwrap();
    assert_eq!(request.parent, 42);
    assert_eq!(request.root, "C:/allowed folder/");
    assert!(parse(&[OsString::from("--ordinary-gui")]).is_none());
    for invalid in [
        "C:relative",
        r"\\server\share",
        r"\\.\C:",
        "C:/root/../secret",
        "C:/x:stream",
        "C:/x/./y",
        "C:/root//child",
    ] {
        assert!(validate_root(invalid).is_err(), "{invalid}");
        let mut malformed = args.clone();
        malformed[4] = invalid.into();
        assert!(parse(&malformed).unwrap().is_err());
    }
    let mut surplus = args.clone();
    surplus.push("--execute=something".into());
    assert!(parse(&surplus).unwrap().is_err());
    assert!(parse(&[OsString::from("--storage-analysis-admin=C:/")])
        .unwrap()
        .is_err());
    assert!(contains("C:/allowed", "c:\\allowed\\child"));
    assert!(!contains("C:/allowed", "C:/allowed-other/child"));
    assert!(!contains("C:/allowed", "C:/allowed/../secret"));
    assert!(
        serde_json::from_str::<ReadRequest>(r#"{"path":"C:/allowed/a","kind":"File"}"#).is_ok()
    );
    for malformed in [
        r#"{"path":"C:/allowed/a","kind":"Write"}"#,
        r#"{"path":"C:/allowed/a","kind":"File","command":"run"}"#,
    ] {
        assert!(serde_json::from_str::<ReadRequest>(malformed).is_err());
    }
    assert_eq!(quote("C:\\with space\\"), "\"C:\\with space\\\\\"");
    assert_eq!(quote("a\"b"), "\"a\\\"b\"");
}
