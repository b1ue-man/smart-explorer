//! Locator grammar and cleartext paths of the vault layer (VLT-M3, stage
//! `vault_unit`).
use super::locator::{
    is_id_prefix, is_site_id, is_vault_locator, vault_location, vault_store_location, VaultLocator,
    VaultStoreLocator,
};
use super::path_map::{components, dir_key, key_within, storage_path};

const SITE: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn vault_task_locator_parse_roundtrip() {
    let cases = [
        (
            format!("vault://{SITE}/Docs/a b.txt"),
            SITE,
            "/Docs/a b.txt",
        ),
        (format!("vault://{SITE}/"), SITE, "/"),
        (format!("vault://{SITE}"), SITE, "/"),
        // `<SITE>` as label or ID prefix; the path stays literal.
        (
            "vault://Mein Tresor/%20x".to_string(),
            "Mein Tresor",
            "/%20x",
        ),
        ("vault://abcd/x//y".to_string(), "abcd", "/x//y"),
    ];
    for (text, site, path) in &cases {
        let parsed = VaultLocator::parse(text).unwrap();
        assert_eq!((parsed.site.as_str(), parsed.path.as_str()), (*site, *path));
        assert_eq!(VaultLocator::parse(&parsed.location()).unwrap(), parsed);
    }
    assert_eq!(
        VaultLocator::parse(&format!("vault://{SITE}"))
            .unwrap()
            .location(),
        format!("vault://{SITE}/")
    );
    assert_eq!(VaultLocator::parse("VAULT://abcd/x").unwrap().site, "abcd");
    for bad in [
        "vault://",
        "vault:///x",
        "vault://a\0b/x",
        "vault-store://abcd",
        "sftp://h/x",
        "vault:x",
    ] {
        assert!(VaultLocator::parse(bad).is_err(), "{bad:?}");
    }
    assert_eq!(vault_location(SITE, "Docs"), format!("vault://{SITE}/Docs"));
    assert_eq!(vault_location(SITE, ""), format!("vault://{SITE}/"));
    // The storage view: full ID only, no path.
    let store = VaultStoreLocator::parse(&format!("vault-store://{SITE}")).unwrap();
    assert_eq!(store.site, SITE);
    assert_eq!(store.location(), format!("vault-store://{SITE}"));
    assert_eq!(store.location(), vault_store_location(SITE));
    let trailing = VaultStoreLocator::parse(&format!("vault-store://{SITE}/")).unwrap();
    assert_eq!(trailing, store);
    let upper = format!("VAULT-STORE://{}", SITE.to_uppercase());
    assert_eq!(VaultStoreLocator::parse(&upper).unwrap(), store);
    for bad in [
        format!("vault-store://{SITE}/x"),
        "vault-store://abcd".to_string(),
        "vault-store://Mein Tresor".to_string(),
        format!("vault://{SITE}"),
    ] {
        assert!(VaultStoreLocator::parse(&bad).is_err(), "{bad}");
    }
    assert!(is_site_id(SITE) && !is_site_id(&SITE.to_uppercase()) && !is_site_id(&SITE[..31]));
    assert!(is_id_prefix("abcd") && is_id_prefix("ABCD") && !is_id_prefix("abc"));
    assert!(!is_id_prefix("wxyz"));
    assert!(is_vault_locator(" vault://x/") && is_vault_locator("Vault-Store://x"));
    assert!(!is_vault_locator("vaults://x") && !is_vault_locator("/vault/x"));
    // Cleartext paths: literal components, never `.` or `..`; NFC keys.
    assert_eq!(components("/a//b/").unwrap(), ["a", "b"]);
    assert!(components("").unwrap().is_empty());
    assert!(components("/a/../b").is_err() && components("/a/./b").is_err());
    assert!(components("/a\0").is_err());
    assert_eq!(dir_key(&["Gru\u{308}n"]), dir_key(&["Gr\u{fc}n"]));
    assert_eq!(dir_key(&[]), "/");
    assert!(key_within("/a/b", "/a") && key_within("/a", "/") && !key_within("/ab", "/a"));
    assert_eq!(storage_path("/srv/Tresor/", "d/AB"), "/srv/Tresor/d/AB");
    assert_eq!(storage_path("/", "d/AB"), "/d/AB");
    assert_eq!(storage_path("C:/Tresor", ""), "C:/Tresor");
}
