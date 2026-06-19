use super::core::{psk_from_code, sanitize_name};
use super::gen_code;
use super::wire::{Ctrl, FileMeta};

#[test]
fn psk_is_deterministic_per_code_and_differs() {
    assert_eq!(psk_from_code("ABC123"), psk_from_code("abc123 "));
    assert_ne!(psk_from_code("ABC123"), psk_from_code("XYZ789"));
    assert_eq!(psk_from_code("K7P2QX9F").len(), 32);
}

#[test]
fn code_is_8_unambiguous_chars() {
    let c = gen_code();
    assert_eq!(c.len(), 8);
    assert!(c.chars().all(|ch| "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(ch)));
}

#[test]
fn sanitize_strips_separators() {
    assert_eq!(sanitize_name("../e/t\\c:passwd"), "_e_t_c_passwd");
    assert_eq!(sanitize_name(""), "datei");
}

#[test]
fn ctrl_roundtrips() {
    let o = Ctrl::Offer { from: "A".into(), files: vec![FileMeta { name: "x".into(), size: 3 }] };
    let j = serde_json::to_vec(&o).unwrap();
    assert!(matches!(serde_json::from_slice::<Ctrl>(&j).unwrap(), Ctrl::Offer { .. }));
}
