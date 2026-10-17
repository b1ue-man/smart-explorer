//! `vault.cryptomator` (format 8): a JWT (JWS compact) whose header names the
//! key source (`kid`) and whose claims carry `format`, `cipherCombo`,
//! `shorteningThreshold` and the vault ID `jti`, signed with HMAC (HS256,
//! HS384 or HS512) under the raw masterkey encryption key ‖ MAC key.
//! Opening follows the order of the format documentation: decode without
//! trust, check `kid` (only the fixed key file, C1.4), load the key, verify
//! the signature in constant time, only then check format and cipher combo.
use std::io::Read;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hmac_013::{Hmac, KeyInit, Mac};
use serde_json::{json, Map, Value};

use super::cipher_combo::CipherCombo;
use super::dir_ids::new_dir_id;
use super::keys::Masterkey;
use super::vault_files::{
    read_bounded, MASTERKEY_FILE_KEY_ID, MAX_SHORTENING_THRESHOLD, METADATA_READ_LIMIT,
    MIN_SHORTENING_THRESHOLD,
};
use super::FormatError;

/// The only vault format se reads and writes.
pub(crate) const VAULT_FORMAT: i64 = 8;

/// Signature algorithms CryptoFS accepts; it writes HS256.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JwtAlg {
    Hs256,
    Hs384,
    Hs512,
}

impl JwtAlg {
    fn parse(alg: &str) -> Option<Self> {
        match alg {
            "HS256" => Some(Self::Hs256),
            "HS384" => Some(Self::Hs384),
            "HS512" => Some(Self::Hs512),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Hs256 => "HS256",
            Self::Hs384 => "HS384",
            Self::Hs512 => "HS512",
        }
    }
}

/// Where the masterkey of a vault comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeySource {
    /// `masterkey.cryptomator` in the vault root, never another path.
    MasterkeyFile,
}

/// A decoded but not yet trusted vault configuration.
#[derive(Clone, Debug)]
pub(crate) struct UnverifiedConfig {
    signing_input: String,
    signature: Vec<u8>,
    alg: Option<String>,
    kid: Option<String>,
    claims: Map<String, Value>,
}

/// A verified vault configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VaultConfig {
    /// The vault ID (`jti`).
    pub(crate) jti: String,
    pub(crate) combo: CipherCombo,
    /// Encrypted names longer than this are shortened (`.c9s`).
    pub(crate) shortening_threshold: u32,
}

fn invalid(detail: &str) -> FormatError {
    FormatError::InvalidConfig(detail.to_string())
}

fn decode_part(part: &str) -> Result<Vec<u8>, FormatError> {
    URL_SAFE_NO_PAD
        .decode(part.trim_end_matches('=').as_bytes())
        .map_err(|_| invalid("kein Base64url"))
}

fn decode_object(part: &str) -> Result<Map<String, Value>, FormatError> {
    match serde_json::from_slice(&decode_part(part)?) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(invalid("kein JSON-Objekt")),
    }
}

fn text(map: &Map<String, Value>, name: &str) -> Option<String> {
    map.get(name).and_then(Value::as_str).map(str::to_string)
}

impl UnverifiedConfig {
    /// Decodes the token without trusting anything in it.
    pub(crate) fn parse(token: &str) -> Result<Self, FormatError> {
        let token = token.trim();
        let mut parts = token.split('.');
        let (Some(header), Some(payload), Some(signature), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(invalid("kein JWT aus drei Teilen"));
        };
        let header_map = decode_object(header)?;
        let claims = decode_object(payload)?;
        let signature = decode_part(signature)?;
        if signature.is_empty() {
            return Err(FormatError::ConfigSignature);
        }
        Ok(Self {
            signing_input: format!("{header}.{payload}"),
            signature,
            alg: text(&header_map, "alg"),
            kid: text(&header_map, "kid"),
            claims,
        })
    }

    /// Reads and decodes `vault.cryptomator`; reads at most 64 KiB plus one
    /// byte (same reasoning as the key file limit, C1.3).
    pub(crate) fn read_from(reader: impl Read) -> Result<Self, FormatError> {
        match read_bounded(reader, METADATA_READ_LIMIT) {
            Ok(Some(bytes)) => {
                let token = String::from_utf8(bytes).map_err(|_| invalid("kein UTF-8"))?;
                Self::parse(&token)
            }
            Ok(None) => Err(FormatError::ConfigTooLarge),
            Err(error) => Err(invalid(&format!("nicht lesbar ({})", error.kind()))),
        }
    }

    /// The key source named by `kid` (C1.4): only
    /// `masterkeyfile:masterkey.cryptomator`; Hub keys and every other
    /// value are refused before anything is loaded.
    pub(crate) fn key_source(&self) -> Result<KeySource, FormatError> {
        match self.kid.as_deref() {
            Some(MASTERKEY_FILE_KEY_ID) => Ok(KeySource::MasterkeyFile),
            Some(kid) if kid.starts_with("hub+") => Err(FormatError::HubVault),
            _ => Err(FormatError::UnsupportedKeySource),
        }
    }

    /// The vault ID as written, before verification (CryptoFS
    /// `allegedVaultId`): enough to tell two vaults apart without a key.
    pub(crate) fn alleged_vault_id(&self) -> Option<String> {
        text(&self.claims, "jti")
    }

    /// The format as written, before verification.
    pub(crate) fn alleged_format(&self) -> Option<i64> {
        self.claims.get("format").and_then(Value::as_i64)
    }

    /// Verifies the signature with the vault's masterkey, then the format
    /// and the claims.
    pub(crate) fn verify(&self, key: &Masterkey) -> Result<VaultConfig, FormatError> {
        self.key_source()?;
        let alg = self
            .alg
            .as_deref()
            .and_then(JwtAlg::parse)
            .ok_or_else(|| invalid("Signaturverfahren nicht unterstützt"))?;
        let raw = key.raw();
        if !signature_matches(
            alg,
            &raw[..],
            self.signing_input.as_bytes(),
            &self.signature,
        )? {
            return Err(FormatError::ConfigSignature);
        }
        let format = self
            .alleged_format()
            .ok_or_else(|| invalid("format fehlt"))?;
        if format != VAULT_FORMAT {
            return Err(FormatError::UnsupportedFormat(format));
        }
        let combo = text(&self.claims, "cipherCombo")
            .as_deref()
            .and_then(CipherCombo::parse)
            .ok_or_else(|| invalid("Verschlüsselungsart nicht unterstützt"))?;
        let jti = self
            .alleged_vault_id()
            .filter(|jti| !jti.is_empty())
            .ok_or_else(|| invalid("jti fehlt"))?;
        let shortening_threshold = self
            .claims
            .get("shorteningThreshold")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or_else(|| invalid("shorteningThreshold fehlt oder ungültig"))?;
        Ok(VaultConfig {
            jti,
            combo,
            shortening_threshold,
        })
    }
}

impl VaultConfig {
    /// The configuration of a new vault (C1.6): `SIV_GCM`, a random vault
    /// ID and a `shorteningThreshold` from 36 to 220.
    pub(crate) fn create(shortening_threshold: u32) -> Result<Self, FormatError> {
        if !(MIN_SHORTENING_THRESHOLD..=MAX_SHORTENING_THRESHOLD).contains(&shortening_threshold) {
            return Err(invalid("shorteningThreshold außerhalb von 36 bis 220"));
        }
        Ok(Self {
            jti: new_dir_id()?,
            combo: CipherCombo::SivGcm,
            shortening_threshold,
        })
    }

    /// The token of this configuration, signed with HS256 as CryptoFS does.
    pub(crate) fn sign(&self, key: &Masterkey) -> Result<String, FormatError> {
        let header = json!({ "kid": MASTERKEY_FILE_KEY_ID, "typ": "JWT", "alg": "HS256" });
        let claims = json!({
            "jti": self.jti,
            "format": VAULT_FORMAT,
            "cipherCombo": self.combo.as_str(),
            "shorteningThreshold": self.shortening_threshold,
        });
        encode_token(JwtAlg::Hs256, &header.to_string(), &claims.to_string(), key)
    }
}

/// Signs the JSON texts `header` and `claims` with `alg` (token creation
/// and the algorithm and tamper checks of the tests).
pub(super) fn encode_token(
    alg: JwtAlg,
    header: &str,
    claims: &str,
    key: &Masterkey,
) -> Result<String, FormatError> {
    let signing_input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.as_bytes()),
        URL_SAFE_NO_PAD.encode(claims.as_bytes())
    );
    let raw = key.raw();
    let signature = match alg {
        JwtAlg::Hs256 => sign::<Hmac<sha2_011::Sha256>>(&raw[..], signing_input.as_bytes())?,
        JwtAlg::Hs384 => sign::<Hmac<sha2_011::Sha384>>(&raw[..], signing_input.as_bytes())?,
        JwtAlg::Hs512 => sign::<Hmac<sha2_011::Sha512>>(&raw[..], signing_input.as_bytes())?,
    };
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

fn sign<M: Mac + KeyInit>(key: &[u8], input: &[u8]) -> Result<Vec<u8>, FormatError> {
    let mut mac = <M as KeyInit>::new_from_slice(key).map_err(|_| FormatError::Crypto("HMAC"))?;
    Mac::update(&mut mac, input);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn matches<M: Mac + KeyInit>(key: &[u8], input: &[u8], tag: &[u8]) -> Result<bool, FormatError> {
    let mut mac = <M as KeyInit>::new_from_slice(key).map_err(|_| FormatError::Crypto("HMAC"))?;
    Mac::update(&mut mac, input);
    Ok(mac.verify_slice(tag).is_ok())
}

/// Constant-time check of the JWT signature (`Mac::verify_slice`).
fn signature_matches(
    alg: JwtAlg,
    key: &[u8],
    input: &[u8],
    signature: &[u8],
) -> Result<bool, FormatError> {
    match alg {
        JwtAlg::Hs256 => matches::<Hmac<sha2_011::Sha256>>(key, input, signature),
        JwtAlg::Hs384 => matches::<Hmac<sha2_011::Sha384>>(key, input, signature),
        JwtAlg::Hs512 => matches::<Hmac<sha2_011::Sha512>>(key, input, signature),
    }
}
