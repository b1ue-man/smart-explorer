//! `vault.cryptomator` (signature algorithms, tampering, key source C1.4)
//! and the backup names of the key file and the configuration.
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;

use super::cipher_combo::CipherCombo;
use super::keys::Masterkey;
use super::vault_config::{encode_token, JwtAlg, KeySource, UnverifiedConfig, VaultConfig};
use super::vault_files::{backup_file_name, backup_suffix, is_metadata_name};
use super::FormatError;

const KID: &str = "masterkeyfile:masterkey.cryptomator";
const JTI: &str = "ea3282b3-3847-499b-82fd-a3723857a225";
const CLAIMS: &str = r#"{"jti":"ea3282b3-3847-499b-82fd-a3723857a225","format":8,"cipherCombo":"SIV_GCM","shorteningThreshold":220}"#;

fn key() -> Masterkey {
    let raw: [u8; 64] = std::array::from_fn(|index| index as u8);
    Masterkey::from_raw(&raw)
}

fn token(alg: JwtAlg, alg_name: &str, kid: &str, claims: &str, key: &Masterkey) -> String {
    let header = format!(r#"{{"kid":"{kid}","typ":"JWT","alg":"{alg_name}"}}"#);
    encode_token(alg, &header, claims, key).expect("Token")
}

fn parse(token: &str) -> UnverifiedConfig {
    UnverifiedConfig::parse(token).expect("JWT")
}

#[test]
fn vault_task_jwt_algs_and_tamper() {
    let key = key();
    let expected = VaultConfig {
        jti: JTI.to_string(),
        combo: CipherCombo::SivGcm,
        shortening_threshold: 220,
    };
    // CryptoFS reads HS256, HS384 and HS512.
    for (alg, name) in [
        (JwtAlg::Hs256, "HS256"),
        (JwtAlg::Hs384, "HS384"),
        (JwtAlg::Hs512, "HS512"),
    ] {
        let config = parse(&token(alg, name, KID, CLAIMS, &key));
        assert_eq!(config.verify(&key).unwrap(), expected, "{name}");
    }
    let ctrmac = CLAIMS.replace("SIV_GCM", "SIV_CTRMAC");
    let config = parse(&token(JwtAlg::Hs256, "HS256", KID, &ctrmac, &key));
    assert_eq!(config.verify(&key).unwrap().combo, CipherCombo::SivCtrMac);

    // The header's algorithm must match the signature.
    let mixed = parse(&token(JwtAlg::Hs256, "HS512", KID, CLAIMS, &key));
    assert_eq!(
        mixed.verify(&key).unwrap_err(),
        FormatError::ConfigSignature
    );
    for name in ["none", "RS256", "hs256", ""] {
        let config = parse(&token(JwtAlg::Hs256, name, KID, CLAIMS, &key));
        assert!(
            matches!(config.verify(&key), Err(FormatError::InvalidConfig(_))),
            "{name}"
        );
    }

    // Changed claims under the old signature, a changed signature, a
    // foreign key: never accepted.
    let good = token(JwtAlg::Hs256, "HS256", KID, CLAIMS, &key);
    let parts: Vec<&str> = good.split('.').collect();
    let forged_claims = URL_SAFE_NO_PAD.encode(ctrmac.as_bytes());
    let forged = format!("{}.{forged_claims}.{}", parts[0], parts[2]);
    assert_eq!(
        parse(&forged).verify(&key).unwrap_err(),
        FormatError::ConfigSignature
    );
    let mut signature = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
    signature[0] ^= 0x01;
    let flipped = format!(
        "{}.{}.{}",
        parts[0],
        parts[1],
        URL_SAFE_NO_PAD.encode(&signature)
    );
    assert_eq!(
        parse(&flipped).verify(&key).unwrap_err(),
        FormatError::ConfigSignature
    );
    let other = Masterkey::from_raw(&[7u8; 64]);
    assert_eq!(
        parse(&good).verify(&other).unwrap_err(),
        FormatError::ConfigSignature
    );
    let unsigned = format!("{}.{}.", parts[0], parts[1]);
    for broken in ["", "a.b", "a.b.c.d", unsigned.as_str()] {
        assert!(UnverifiedConfig::parse(broken).is_err(), "{broken}");
    }

    // Format and claims are checked after the signature.
    let format7 = CLAIMS.replace("\"format\":8", "\"format\":7");
    let error = parse(&token(JwtAlg::Hs256, "HS256", KID, &format7, &key))
        .verify(&key)
        .unwrap_err();
    assert_eq!(error, FormatError::UnsupportedFormat(7));
    assert!(error.to_string().contains("in Cryptomator"));
    let unknown_combo = CLAIMS.replace("SIV_GCM", "SIV_CBC");
    assert!(matches!(
        parse(&token(JwtAlg::Hs256, "HS256", KID, &unknown_combo, &key)).verify(&key),
        Err(FormatError::InvalidConfig(_))
    ));

    // A new configuration: SIV_GCM, random UUIDv4 vault ID, threshold 36 to
    // 220, signed with HS256 and readable again.
    let created = VaultConfig::create(220).unwrap();
    assert_eq!(created.combo, CipherCombo::SivGcm);
    assert_eq!(created.jti.len(), 36);
    assert_eq!(&created.jti[14..15], "4");
    let signed = created.sign(&key).unwrap();
    assert_eq!(parse(&signed).verify(&key).unwrap(), created);
    assert_ne!(VaultConfig::create(220).unwrap().jti, created.jti);
    assert!(VaultConfig::create(36).is_ok());
    assert!(VaultConfig::create(35).is_err());
    assert!(VaultConfig::create(221).is_err());
}

#[test]
fn vault_task_kid_only_fixed_masterkeyfile() {
    let key = key();
    let accepted = parse(&token(JwtAlg::Hs256, "HS256", KID, CLAIMS, &key));
    assert_eq!(accepted.key_source(), Ok(KeySource::MasterkeyFile));
    // The vault ID can be compared before any key is loaded.
    assert_eq!(accepted.alleged_vault_id().as_deref(), Some(JTI));
    assert_eq!(accepted.alleged_format(), Some(8));
    // Any other path is refused, also for an otherwise valid signature.
    for kid in [
        "masterkeyfile:../masterkey.cryptomator",
        "masterkeyfile:/etc/passwd",
        "masterkeyfile:masterkey.cryptomator/../../secret",
        "masterkeyfile:other.cryptomator",
        "masterkeyfile:MASTERKEY.CRYPTOMATOR",
        "file:masterkey.cryptomator",
        "",
    ] {
        let config = parse(&token(JwtAlg::Hs256, "HS256", kid, CLAIMS, &key));
        assert_eq!(
            config.key_source(),
            Err(FormatError::UnsupportedKeySource),
            "{kid}"
        );
        assert_eq!(
            config.verify(&key).unwrap_err(),
            FormatError::UnsupportedKeySource,
            "{kid}"
        );
    }
    let hub = parse(&token(
        JwtAlg::Hs256,
        "HS256",
        "hub+https://hub.example.com/api/vaults/1",
        CLAIMS,
        &key,
    ));
    assert_eq!(hub.key_source(), Err(FormatError::HubVault));
    assert_eq!(
        FormatError::HubVault.to_string(),
        "Cryptomator-Hub-Tresore werden nicht unterstützt"
    );
    let without_kid = encode_token(
        JwtAlg::Hs256,
        r#"{"typ":"JWT","alg":"HS256"}"#,
        CLAIMS,
        &key,
    )
    .unwrap();
    assert_eq!(
        parse(&without_kid).key_source(),
        Err(FormatError::UnsupportedKeySource)
    );
}

#[test]
fn vault_task_backup_suffix_uppercase() {
    // "." + the first 4 bytes of SHA-256 as uppercase hex (Guava base16).
    assert_eq!(backup_suffix(b"abc"), ".BA7816BF");
    assert_eq!(backup_suffix(b""), ".E3B0C442");
    assert_eq!(
        backup_file_name("masterkey.cryptomator", b"abc"),
        "masterkey.cryptomator.BA7816BF.bkup"
    );
    for content in [&b"x"[..], &[0xFFu8; 1000][..], &b"{\"version\":999}"[..]] {
        let suffix = backup_suffix(content);
        assert_eq!(suffix.len(), 9);
        assert!(suffix.starts_with('.'));
        assert!(suffix[1..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte)));
    }
    assert!(is_metadata_name("vault.cryptomator"));
    assert!(is_metadata_name("masterkey.cryptomator"));
    assert!(is_metadata_name("masterkey.cryptomator.BA7816BF.bkup"));
    assert!(is_metadata_name("vault.cryptomator.1A2B3C4D.bkup"));
    for other in [
        "masterkey.cryptomator.ba7816bf.bkup",
        "masterkey.cryptomator.BA7816B.bkup",
        "masterkey.cryptomator.bkup",
        "IMPORTANT.rtf",
        "d",
    ] {
        assert!(!is_metadata_name(other), "{other}");
    }
}
