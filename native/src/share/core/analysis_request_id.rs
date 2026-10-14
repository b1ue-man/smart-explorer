//! `request_id` of `StorageAnalysis`/`DuplicateSearch` (`analysis_reattach_v1`):
//! the key under which a host keeps a job for a later reattach.
//!
//! Clients send 32 lowercase hex digits, the form every host since 0.5.170
//! accepts. Hosts also accept the URL-safe base64 tokens (22 characters) that
//! clients 0.5.170 to 0.5.177 sent; those hosts refused them, so no remote
//! analysis worked between such versions. The id only keys the host's job
//! table together with the peer principal; it never becomes a path or a
//! command.

/// A fresh id for one analysis or duplicate search.
pub(crate) fn new() -> Result<String, String> {
    super::core::random_hex_token::<16>()
}

/// Whether a host keeps a job under `id`.
pub(crate) fn is_valid(id: &str) -> bool {
    (16..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::{is_valid, new};

    #[test]
    fn analysis_id_task_new_and_released_ids_are_accepted() {
        for _ in 0..64 {
            let id = new().expect("random id");
            assert_eq!(id.len(), 32, "{id}");
            assert!(
                id.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "{id}"
            );
            assert!(is_valid(&id), "{id}");
            // The form clients 0.5.170 to 0.5.177 sent.
            let released = crate::share::core::random_token(16).expect("random token");
            assert!(is_valid(&released), "{released}");
        }
        for refused in [
            "",
            "0123456789abcde",
            "../../etc/passwd/x",
            "0123456789abcdef 0",
            "0123456789abcdef/0",
            "0123456789abcdef.0",
            "0123456789abcdéf0",
        ] {
            assert!(!is_valid(refused), "{refused}");
        }
        assert!(is_valid(&"a".repeat(64)));
        assert!(!is_valid(&"a".repeat(65)));
    }
}
