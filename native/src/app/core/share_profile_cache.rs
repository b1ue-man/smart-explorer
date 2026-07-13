use super::*;

pub(super) fn reload(app: &mut App) -> Result<(), String> {
    let home = dirs_home().to_string_lossy().replace('\\', "/");
    let mut profiles = crate::share::ShareProfiles::load_checked(Some(home.clone()))?;
    if !profiles.legacy_direct_requests.is_empty() {
        let identity = app
            .share_identity
            .as_ref()
            .ok_or_else(|| "Share-Identitaet nicht verfuegbar".to_string())?;
        profiles = crate::share::refresh_legacy_request_expiry(Some(home), identity)?;
    }
    app.share_profiles = profiles;
    app.share_profiles_error = None;
    if app
        .share_profiles
        .legacy_direct_requests
        .iter()
        .any(|entry| entry.is_pending(crate::share::core_now_secs()))
    {
        app.show_share = true;
        app.share_tab = 0;
    }
    Ok(())
}
