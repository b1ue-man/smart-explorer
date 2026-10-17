//! The two vault locators (C2.2):
//!
//! - `vault://<SITE>/<path>`: the cleartext view of a registered vault
//!   site. `<SITE>` is written as the site ID, an ID prefix of at least 4
//!   characters or a unique label; stored locators always carry the ID
//!   (`canonical`). The path is literal (no decoding), like every se
//!   locator; an empty path is the vault root `/`.
//! - `vault-store://<site-id>`: the encrypted storage of a site, without a
//!   path part, for vault sync jobs. It resolves without a wrapper to the
//!   site's own location (C2.3).
//!
//! A locator names a site of the vault list, never a path of the storage,
//! so it can never be taken for a local path.

/// Scheme of the cleartext view.
pub(crate) const VAULT_SCHEME: &str = "vault";
/// Scheme of the encrypted storage view.
pub(crate) const VAULT_STORE_SCHEME: &str = "vault-store";
/// Hex digits of a site ID.
pub(crate) const SITE_ID_LEN: usize = 32;
/// Shortest ID prefix accepted as `<SITE>`.
pub(crate) const MIN_SITE_PREFIX: usize = 4;

/// Whether `text` is a site ID as se writes it: 32 lowercase hex digits.
pub(crate) fn is_site_id(text: &str) -> bool {
    text.len() == SITE_ID_LEN
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Whether `text` can be (a prefix of) a site ID: 4 to 32 hex digits, any
/// letter case.
pub(crate) fn is_id_prefix(text: &str) -> bool {
    (MIN_SITE_PREFIX..=SITE_ID_LEN).contains(&text.len())
        && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// `rest` after `<scheme>://` when `locator` has exactly this scheme (any
/// letter case, as `connect::EndpointSpec` compares schemes).
fn after_scheme<'a>(locator: &'a str, scheme: &str) -> Option<&'a str> {
    let (given, rest) = locator.split_once("://")?;
    given.eq_ignore_ascii_case(scheme).then_some(rest)
}

/// Whether `locator` uses the scheme `vault://` or `vault-store://`.
pub(crate) fn is_vault_locator(locator: &str) -> bool {
    let locator = locator.trim_start();
    after_scheme(locator, VAULT_SCHEME).is_some()
        || after_scheme(locator, VAULT_STORE_SCHEME).is_some()
}

/// `vault://<SITE>/<path>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VaultLocator {
    /// The site as written (ID, ID prefix or label).
    pub(crate) site: String,
    /// Absolute cleartext path inside the vault, kept literally.
    pub(crate) path: String,
}

impl VaultLocator {
    pub(crate) fn parse(locator: &str) -> Result<Self, String> {
        let rest =
            after_scheme(locator, VAULT_SCHEME).ok_or_else(|| "Kein vault://-Ort".to_string())?;
        if rest.contains('\0') {
            return Err("Ungültiger Tresorort".into());
        }
        let (site, path) = match rest.find('/') {
            Some(index) => rest.split_at(index),
            None => (rest, "/"),
        };
        if site.is_empty() {
            return Err("vault:// braucht einen Tresorort (ID, ID-Anfang oder Bezeichnung)".into());
        }
        Ok(Self {
            site: site.to_string(),
            path: path.to_string(),
        })
    }

    /// `vault://<site><path>`.
    pub(crate) fn location(&self) -> String {
        vault_location(&self.site, &self.path)
    }
}

/// `vault-store://<site-id>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VaultStoreLocator {
    /// The site ID (lowercase).
    pub(crate) site: String,
}

impl VaultStoreLocator {
    pub(crate) fn parse(locator: &str) -> Result<Self, String> {
        let rest = after_scheme(locator, VAULT_STORE_SCHEME)
            .ok_or_else(|| "Kein vault-store://-Ort".to_string())?;
        let site = rest.strip_suffix('/').unwrap_or(rest);
        if site.contains('/') {
            return Err("vault-store:// hat keinen Pfadteil".into());
        }
        let site = site.to_ascii_lowercase();
        if !is_site_id(&site) {
            return Err(
                "vault-store:// braucht die vollständige Tresorort-ID (32 Hexziffern)".into(),
            );
        }
        Ok(Self { site })
    }

    pub(crate) fn location(&self) -> String {
        vault_store_location(&self.site)
    }
}

/// `vault://<site><path>`; `path` empty means the vault root.
pub(crate) fn vault_location(site: &str, path: &str) -> String {
    if path.is_empty() {
        format!("{VAULT_SCHEME}://{site}/")
    } else if path.starts_with('/') {
        format!("{VAULT_SCHEME}://{site}{path}")
    } else {
        format!("{VAULT_SCHEME}://{site}/{path}")
    }
}

/// `vault-store://<site-id>`.
pub(crate) fn vault_store_location(site: &str) -> String {
    format!("{VAULT_STORE_SCHEME}://{site}")
}
