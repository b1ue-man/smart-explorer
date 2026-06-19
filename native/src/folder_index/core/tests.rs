use super::search::fuzzy_score;

fn s(s: &str) -> i32 {
    fuzzy_score(s.to_lowercase().as_bytes(), s.as_bytes()).unwrap_or(0)
}

#[test]
fn basic() {
    // Identical match scores higher than substring
    assert!(s("Downloads") > 0);
    // "dnlds" matches "Downloads" but lower than "downloads"
    let exact = fuzzy_score(b"downloads", b"C:/Users/Silas/Downloads".as_ref()).unwrap();
    let fuzzy = fuzzy_score(b"dnlds", b"C:/Users/Silas/Downloads".as_ref()).unwrap();
    assert!(exact > fuzzy);
}

#[test]
fn no_match() {
    assert!(fuzzy_score(b"xyz", b"abc").is_none());
}
