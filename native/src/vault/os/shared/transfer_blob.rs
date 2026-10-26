//! Durable transfer records: authenticated purpose, record and exact vault scope.
use super::transfer_blob_context::{
    context_for_sites, invalid, validate_context, TransferBlobContext,
};
use crate::vault::MAX_SEALED_TEXT;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use std::io;
use zeroize::Zeroizing;

#[path = "transfer_blob_codec.rs"]
mod codec;
pub(crate) use codec::TransferBlobPurpose;
use codec::{base64_size, chunk_capacity, sealed_text_size, Chunk, Envelope, Payload};

/// `context` must be captured before transferring. No plaintext fallback is
/// valid when capturing, sealing or opening a vault scope fails.
pub(crate) fn seal_transfer_blob(
    context: &TransferBlobContext,
    purpose: TransferBlobPurpose,
    record_id: &str,
    plain: &[u8],
) -> io::Result<Vec<u8>> {
    validate_context(context)?;
    if record_id.is_empty() {
        return Err(invalid());
    }
    let payload = Payload {
        schema: 1,
        purpose,
        record: record_id.into(),
        bindings: context.bindings.clone(),
        contents: URL_SAFE_NO_PAD.encode(plain),
    };
    let bytes = Zeroizing::new(serde_json::to_vec(&payload).map_err(|_| invalid())?);
    let blob = crate::vault::new_site_id().map_err(|_| invalid())?;
    let sites = context
        .bindings
        .iter()
        .map(|binding| binding.site.clone())
        .collect::<Vec<_>>();
    let capacity = chunk_capacity(&blob, purpose, sites.len())?;
    let count = bytes.len().div_ceil(capacity);
    let mut chunks = Vec::with_capacity(count);
    for (index, bytes) in bytes.chunks(capacity).enumerate() {
        let frame = Chunk {
            blob: blob.clone(),
            purpose,
            index,
            count,
            bytes: URL_SAFE_NO_PAD.encode(bytes),
        };
        let mut text = Zeroizing::new(serde_json::to_string(&frame).map_err(|_| invalid())?);
        for site in &sites {
            text = Zeroizing::new(seal_text(site, purpose, &text)?);
        }
        chunks.push(text.to_string());
    }
    validate_context(context)?;
    serde_json::to_vec(&Envelope {
        schema: 1,
        purpose,
        blob,
        sites,
        chunks,
    })
    .map_err(|_| invalid())
}

/// Authenticates each chunk before assembly and returns no bytes on a changed
/// purpose, record ID, account, root, site or current unlock state.
pub(crate) fn open_transfer_blob(
    purpose: TransferBlobPurpose,
    record_id: &str,
    sealed: &[u8],
    max_plaintext_bytes: usize,
) -> io::Result<Zeroizing<Vec<u8>>> {
    let envelope: Envelope = serde_json::from_slice(sealed).map_err(|_| invalid())?;
    if envelope.schema != 1
        || envelope.purpose != purpose
        || !crate::vault::is_site_id(&envelope.blob)
        || envelope.chunks.is_empty()
        || record_id.is_empty()
    {
        return Err(invalid());
    }
    let context = context_for_sites(&envelope.sites)?;
    let empty = Payload {
        schema: 1,
        purpose,
        record: record_id.into(),
        bindings: context.bindings.clone(),
        contents: String::new(),
    };
    let limit = serde_json::to_vec(&empty)
        .map_err(|_| invalid())?
        .len()
        .checked_add(base64_size(max_plaintext_bytes)?)
        .ok_or_else(invalid)?;
    let mut assembled = Zeroizing::new(Vec::new());
    for (index, sealed) in envelope.chunks.iter().enumerate() {
        let mut text = Zeroizing::new(sealed.clone());
        for site in envelope.sites.iter().rev() {
            text = unseal_text(site, purpose, &text)?;
        }
        let chunk: Chunk = serde_json::from_str(&text).map_err(|_| invalid())?;
        if chunk.blob != envelope.blob
            || chunk.purpose != purpose
            || chunk.index != index
            || chunk.count != envelope.chunks.len()
        {
            return Err(invalid());
        }
        let bytes = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(&chunk.bytes)
                .map_err(|_| invalid())?,
        );
        if bytes.is_empty()
            || assembled
                .len()
                .checked_add(bytes.len())
                .is_none_or(|length| length > limit)
        {
            return Err(invalid());
        }
        assembled.extend_from_slice(&bytes);
    }
    let payload: Payload = serde_json::from_slice(&assembled).map_err(|_| invalid())?;
    if payload.schema != 1
        || payload.purpose != purpose
        || payload.record != record_id
        || payload.bindings != context.bindings
    {
        return Err(invalid());
    }
    let result = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(&payload.contents)
            .map_err(|_| invalid())?,
    );
    if result.len() > max_plaintext_bytes {
        return Err(invalid());
    }
    // Scope can change while remote identities are being resolved or the
    // worker processes chunks. Do not release the assembled names then.
    validate_context(&context)?;
    let current = context_for_sites(&envelope.sites)?;
    if current.bindings != context.bindings {
        return Err(invalid());
    }
    Ok(result)
}
fn ipc_error(site: &str, error: crate::daemon::VaultIpcError) -> io::Error {
    if error.code.as_deref() == Some(crate::vault::VAULT_LOCKED_CODE) {
        crate::vault::vault_locked(site)
    } else {
        invalid()
    }
}
fn seal_text(site: &str, purpose: TransferBlobPurpose, text: &str) -> io::Result<String> {
    let result = if super::hosts_register() {
        super::register().seal_texts(site, purpose.seal(), &[text])?
    } else {
        crate::daemon::seal_vault_items(site, purpose.wire(), &[text])
            .map_err(|error| ipc_error(site, error))?
    };
    if result.len() != 1 {
        return Err(invalid());
    }
    result.into_iter().next().ok_or_else(invalid)
}
fn unseal_text(
    site: &str,
    purpose: TransferBlobPurpose,
    text: &str,
) -> io::Result<Zeroizing<String>> {
    if text.len() > sealed_text_size(MAX_SEALED_TEXT)? {
        return Err(invalid());
    }
    if super::hosts_register() {
        let result = super::register().unseal_texts(site, purpose.seal(), &[text])?;
        if result.len() != 1 {
            return Err(invalid());
        }
        result.into_iter().next().ok_or_else(invalid)
    } else {
        let result = crate::daemon::unseal_vault_items(site, purpose.wire(), &[text.into()])
            .map_err(|error| ipc_error(site, error))?;
        if result.len() != 1 {
            return Err(invalid());
        }
        result
            .into_iter()
            .next()
            .map(|text| Zeroizing::new(text.expose().into()))
            .ok_or_else(invalid)
    }
}
