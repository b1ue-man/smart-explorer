//! Secure-store helpers of the Share identity (moved out of `identity.rs`
//! unchanged, so that file stays below its size budget).
use super::*;

pub(super) fn save_secret_verified(
    storage: &mut impl IdentityPersistence,
    account: &str,
    secret: &[u8; SECRET_BYTES],
    label: &str,
) -> Result<(), String> {
    let result = storage
        .save_secret(account, &b64(secret))
        .map_err(|error| format!("{label} speichern: {error}"))
        .and_then(|()| match load_optional_secret(storage, account, label)? {
            Some(stored) if stored == *secret => Ok(()),
            Some(_) => Err(format!("{label} wurde mit anderen Bytes gespeichert")),
            None => Err(format!("{label} wurde nicht im sicheren Speicher behalten")),
        });
    if let Err(error) = result {
        let cleanup = cleanup_secret(storage, account, label)
            .into_iter()
            .collect();
        return Err(with_cleanup(error, cleanup));
    }
    Ok(())
}

pub(super) fn cleanup_secret(
    storage: &mut impl IdentityPersistence,
    account: &str,
    label: &str,
) -> Option<String> {
    storage
        .delete_secret(account)
        .err()
        .map(|error| format!("{label}: {error}"))
}

pub(super) fn load_required_secret(
    storage: &mut impl IdentityPersistence,
    account: &str,
    label: &str,
) -> Result<[u8; SECRET_BYTES], String> {
    load_optional_secret(storage, account, label)?
        .ok_or_else(|| format!("{label} fehlt im sicheren Speicher"))
}

pub(super) fn load_optional_secret(
    storage: &mut impl IdentityPersistence,
    account: &str,
    label: &str,
) -> Result<Option<[u8; SECRET_BYTES]>, String> {
    storage
        .load_secret(account)
        .map_err(|error| format!("{label} lesen: {error}"))?
        .map(|raw| decode_secret(&raw, label))
        .transpose()
}

pub(super) fn decode_secret(raw: &str, label: &str) -> Result<[u8; SECRET_BYTES], String> {
    b64_decode(raw)
        .map_err(|error| format!("{label} ist ungueltig: {error}"))?
        .try_into()
        .map_err(|_| format!("{label} hat nicht {SECRET_BYTES} Bytes"))
}

pub(super) fn with_cleanup(error: String, cleanup: Vec<String>) -> String {
    if cleanup.is_empty() {
        error
    } else {
        format!(
            "{error}; Bereinigung fehlgeschlagen: {}",
            cleanup.join(", ")
        )
    }
}
