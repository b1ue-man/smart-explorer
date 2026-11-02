use super::{load_pending, JournalEntry, JOURNAL_VERSION, MAX_JOURNAL_BYTES};

pub(super) fn write_entry(entry: &JournalEntry) -> Result<(), String> {
    entry.validate()?;
    let encoded =
        serde_json::to_vec(entry).map_err(|error| format!("Exec-Grant journal encode: {error}"))?;
    if encoded.len() as u64 > MAX_JOURNAL_BYTES {
        return Err("Exec-Grant journal exceeds its 64 KiB limit".into());
    }
    let destination = crate::daemon::ipc_storage::exec_journal_path()
        .map_err(|error| format!("Exec-Grant journal path: {error}"))?;
    crate::support_dirs::write_private_atomic(&destination, &encoded)
        .map_err(|error| format!("Exec-Grant journal durable private commit: {error}"))?;
    match load_pending()? {
        Some(stored) if stored == *entry => Ok(()),
        Some(_) => Err("Exec-Grant journal verification mismatch".into()),
        None => Err("Exec-Grant journal disappeared after commit".into()),
    }
}

pub(super) fn clear_entry(operation_id: &str) -> Result<(), String> {
    let stored = load_pending()?
        .ok_or_else(|| "Exec-Grant journal disappeared before acknowledgement".to_string())?;
    if stored.operation_id != operation_id {
        return Err("Exec-Grant journal operation changed before acknowledgement".into());
    }
    let path = crate::daemon::ipc_storage::exec_journal_path()
        .map_err(|error| format!("Exec-Grant journal path: {error}"))?;
    unlink_and_sync_with_recovery(
        &stored,
        || {
            std::fs::remove_file(&path)
                .map_err(|error| format!("Exec-Grant journal remove: {error}"))
        },
        || {
            crate::daemon::ipc_storage::sync_exec_journal_directory()
                .map_err(|error| format!("Exec-Grant journal directory sync: {error}"))
        },
        write_entry,
        || match load_pending()? {
            Some(recovered) if recovered == stored => Ok(true),
            Some(_) => Err("restored Exec-Grant journal verification mismatch".into()),
            None => Ok(false),
        },
    )
}

pub(super) fn unlink_and_sync_with_recovery<T, R, S, W, V>(
    stored: &T,
    mut remove: R,
    mut sync_directory: S,
    mut restore: W,
    mut verify_restored: V,
) -> Result<(), String>
where
    R: FnMut() -> Result<(), String>,
    S: FnMut() -> Result<(), String>,
    W: FnMut(&T) -> Result<(), String>,
    V: FnMut() -> Result<bool, String>,
{
    remove()?;
    let sync_error = match sync_directory() {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };

    let restore_error = restore(stored).err();
    let verification = verify_restored();
    let recovery = match (restore_error, verification) {
        (None, Ok(true)) => "recovery journal restored and verified".to_string(),
        (Some(error), Ok(true)) => format!(
            "recovery journal is present and verified, but its durable write reported: {error}"
        ),
        (None, Ok(false)) => "recovery journal restoration was not visible".to_string(),
        (Some(error), Ok(false)) => {
            format!("recovery journal restoration failed: {error}")
        }
        (None, Err(error)) => format!("recovery journal verification failed: {error}"),
        (Some(restore), Err(verify)) => {
            format!("recovery journal restoration failed: {restore}; verification failed: {verify}")
        }
    };
    Err(format!("{sync_error}; {recovery}"))
}

pub(super) fn decode_entry(encoded: &[u8]) -> Result<JournalEntry, String> {
    let mut value: serde_json::Value = serde_json::from_slice(encoded)
        .map_err(|error| format!("Exec-Grant journal decode: {error}"))?;
    if value.get("version").and_then(serde_json::Value::as_u64) == Some(1) {
        migrate_v1_target(&mut value)?;
    }
    serde_json::from_value(value).map_err(|error| format!("Exec-Grant journal decode: {error}"))
}

fn migrate_v1_target(value: &mut serde_json::Value) -> Result<(), String> {
    let principal = value
        .pointer("/mutation/principal")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "legacy Exec-Grant journal has no exact principal".to_string())?;
    let field = |name: &str| {
        principal
            .get(name)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("legacy Exec-Grant journal principal lacks {name}"))
    };
    let relation_kind = field("relation_kind")?;
    let relation_id = field("relation_id")?;
    let pins = serde_json::json!({
        "device_id": field("device_id")?,
        "public_key": field("public_key")?,
        "fingerprint": field("fingerprint")?,
        "node_id": field("node_id")?,
    });
    let target = match relation_kind.as_str() {
        "direct" => serde_json::json!({ "Direct": pins }),
        "room" => {
            let mut fields = pins
                .as_object()
                .cloned()
                .ok_or_else(|| "legacy Exec-Grant journal pins are invalid".to_string())?;
            fields.insert("room_id".into(), serde_json::Value::String(relation_id));
            serde_json::json!({ "RoomMember": fields })
        }
        _ => return Err("legacy Exec-Grant journal relation is unsupported".into()),
    };
    value["mutation"]["target"] = target;
    value["version"] = serde_json::json!(JOURNAL_VERSION);
    Ok(())
}
