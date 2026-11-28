//! The sealed files of the replace journal: content of every size comes back
//! as it was, and a file under another label, with another key, cut short,
//! extended or changed in one byte is refused.
use super::replace_journal_seal::{open_from, seal_bytes, JournalKey};

fn key(byte: u8) -> JournalKey {
    JournalKey::from_bytes([byte; 32])
}

fn open(key: &JournalKey, label: &str, sealed: &[u8]) -> std::io::Result<Vec<u8>> {
    open_from(key, label, &mut &sealed[..], u64::MAX)
}

#[test]
fn replace_task_sealed_journal_files_round_trip_and_refuse_tampering() {
    let long: Vec<u8> = (0..(2 * 1024 * 1024 + 17))
        .map(|index| (index % 251) as u8)
        .collect();
    for plain in [Vec::new(), b"Hallo Welt".to_vec(), long] {
        let sealed = seal_bytes(&key(1), "run-1/0/backup", &plain).unwrap();
        assert_eq!(open(&key(1), "run-1/0/backup", &sealed).unwrap(), plain);
        // Another label, another key.
        assert!(open(&key(1), "run-1/1/backup", &sealed).is_err());
        assert!(open(&key(2), "run-1/0/backup", &sealed).is_err());
        // Cut short, extended, one byte changed.
        assert!(open(&key(1), "run-1/0/backup", &sealed[..sealed.len() - 1]).is_err());
        let mut extended = sealed.clone();
        extended.push(0);
        assert!(open(&key(1), "run-1/0/backup", &extended).is_err());
        let mut changed = sealed.clone();
        let last = changed.len() - 1;
        changed[last] ^= 1;
        assert!(open(&key(1), "run-1/0/backup", &changed).is_err());
        // A limit below the content.
        if !plain.is_empty() {
            assert!(open_from(&key(1), "run-1/0/backup", &mut &sealed[..], 1).is_err());
        }
    }
    // Two seals of the same text differ (own salt each).
    let first = seal_bytes(&key(1), "x", b"gleich").unwrap();
    let second = seal_bytes(&key(1), "x", b"gleich").unwrap();
    assert_ne!(first, second);
    // Dropping the chunks after the first of a long text is refused too.
    let long = vec![b'a'; 1024 * 1024 + 1];
    let sealed = seal_bytes(&key(1), "x", &long).unwrap();
    let first_chunk = 8 + 16 + 5 + 1024 * 1024 + 16;
    assert!(open(&key(1), "x", &sealed[..first_chunk]).is_err());
    let hex = key(9).to_hex();
    assert_eq!(hex.len(), 64);
    assert!(JournalKey::from_hex(&hex).is_ok());
    assert!(JournalKey::from_hex("zz").is_err());
}
