//! Relation access proofs (FC4). A proof is HMAC-SHA256 under the Direct or
//! room secret, so the server learns neither the secret nor a way to derive
//! it. The owner of a Direct lookup leaves the SHA-256 of its proof; a
//! watcher that logged in with a key must show a proof with that hash. Room
//! members are partitioned by the hash of their proof: they only see members
//! of the same partition. Clients without key login (older versions) have no
//! partition and stay visible to everyone, so mixed rooms keep working;
//! `--require-key-login` removes them altogether.

use super::login::{hex_decode, sha256};

/// The SHA-256 of a presented proof (64 hex digits for 32 bytes).
pub(super) fn proof_hash(proof_hex: &str) -> Option<[u8; 32]> {
    if proof_hex.len() != 64 { return None; }
    let proof = hex_decode(proof_hex)?;
    (proof.len() == 32).then(|| sha256(&proof))
}

/// A hash the owner left (64 hex digits).
pub(super) fn parse_hash(hash_hex: &str) -> Option<[u8; 32]> {
    if hash_hex.len() != 64 { return None; }
    hex_decode(hash_hex)?.try_into().ok()
}

/// Whether two room members see each other: same partition, or one of them
/// has none (an older client).
pub(super) fn visible(a: Option<&[u8; 32]>, b: Option<&[u8; 32]>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_hash, proof_hash, visible};
    use crate::login::hex;

    /// Shared test vectors with the app (`signal_publish.rs`): the proof of
    /// secret [7; 32] for Direct lookup "lookup-a" and its hash.
    const ACCESS_PROOF_VECTOR: &str =
        "b58b9881a79cc470538f046c183d9a1e34f56a1470faaea976c54e8098f85a49";
    const ACCESS_HASH_VECTOR: &str =
        "2d2db6518dc0bc5c8d0fe95916db6d07b1a7087d75fd90878af7841cc11653c6";

    #[test]
    fn review_task_access_hash_matches_the_app_vectors() {
        let hash = proof_hash(ACCESS_PROOF_VECTOR).expect("proof");
        assert_eq!(hex(&hash), ACCESS_HASH_VECTOR);
        assert_eq!(parse_hash(ACCESS_HASH_VECTOR), Some(hash));
        assert_eq!(proof_hash("abcd"), None);
        assert_eq!(parse_hash("zz"), None);
    }

    #[test]
    fn review_task_room_partitions_hide_members_without_the_secret() {
        let member = [1; 32];
        let stranger = [2; 32];
        assert!(visible(Some(&member), Some(&member)));
        assert!(!visible(Some(&member), Some(&stranger)));
        assert!(visible(None, Some(&member)));
        assert!(visible(Some(&member), None));
    }
}
