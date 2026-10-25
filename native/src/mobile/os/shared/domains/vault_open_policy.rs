//! Android root consent uses the desktop identity binding, never a copy fallback.
use serde_json::{json, Value};
use super::args::{invalid, str_arg};
use crate::mobile::{ApiError, Runtime};

pub(super) fn handle(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "vault.openPolicy" => show(args),
        "vault.confirmRootTrust" => confirm(args),
        "vault.revokeRootTrust" => revoke(args),
        "vault.providerRelease" => release(rt, args),
        _ => return None,
    })
}

fn show(args: &Value) -> Result<Value, ApiError> {
    let record = crate::vault::RegistryStore::open_default()
        .resolve(str_arg(args, "site")?).map_err(ApiError::not_found)?;
    let stored = crate::vault::stored_policy(&record).map_err(ApiError::internal)?;
    let current = crate::vault::mount_policy(&record.id);
    let settings = super::docs_vault::settings();
    Ok(json!({
        "site": record.id,
        "storedRootTrust": stored.trusted_root.is_some(),
        "storedRootBinding": stored.trusted_root,
        "current": current.as_ref().ok(),
        "resolutionError": current.err(),
        "providerEnabled": settings.provider,
        "providerReleased": settings.vaults.contains(&record.id),
        "unlocked": super::docs_vault::is_unlocked(&record.id),
    }))
}

fn confirm(args: &Value) -> Result<Value, ApiError> {
    let binding: crate::vault::RootBinding = serde_json::from_value(
        args.get("binding").cloned().ok_or_else(|| invalid("Speicheridentität fehlt."))?
    ).map_err(|_| invalid("Speicheridentität ist ungültig."))?;
    if binding.site != str_arg(args, "site")? { return Err(invalid("Tresor und Speicheridentität stimmen nicht überein.")); }
    crate::vault::confirm_root_trust(&binding).map_err(ApiError::internal)?;
    show(args)
}

fn revoke(args: &Value) -> Result<Value, ApiError> {
    crate::vault::set_root_trust(str_arg(args, "site")?, false).map_err(ApiError::internal)?;
    show(args)
}

fn release(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let site = str_arg(args, "site")?;
    // Resolve the registered site, but never require a live provider to withdraw it.
    let record = crate::vault::RegistryStore::open_default().resolve(site).map_err(ApiError::not_found)?;
    let enabled = args.get("enabled").and_then(Value::as_bool).ok_or_else(|| invalid("enabled muss bool sein."))?;
    let mut settings = super::docs_vault::settings();
    if enabled {
        if !super::docs_vault::is_unlocked(&record.id) { return Err(invalid("Den Tresor vor der Freigabe entsperren.")); }
        let policy = crate::vault::mount_policy(&record.id).map_err(ApiError::internal)?;
        if !policy.allowed() { return Err(invalid("Speichervertrauen zuerst ausdrücklich bestätigen.")); }
        settings.provider = true;
        if !settings.vaults.contains(&record.id) { settings.vaults.push(record.id.clone()); }
    } else { settings.vaults.retain(|id| id != &record.id); }
    super::docs_vault::set_settings(rt, &json!({ "provider": settings.provider, "vaults": settings.vaults }))?;
    show(args)
}
