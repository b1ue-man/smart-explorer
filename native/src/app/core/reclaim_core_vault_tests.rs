//! K21 of the vault plan: reclaim journal lines of a vault drive keep no
//! cleartext path.
use super::*;

#[test]
fn reclaim_journal_seals_paths_in_vault_drives() {
    let line = || {
        serde_json::json!({
            "root": "/run/tresor/Geheim",
            "selected": ["/run/tresor/Geheim/a"],
            "delete_paths": ["/run/tresor/Geheim/a"],
            "errors": [["/run/tresor/Geheim/b", "Geheim: belegt"]],
            "attempted": 1,
        })
    };
    let seal = |site: &str, items: &[&str]| -> Result<Vec<String>, String> {
        Ok(items
            .iter()
            .map(|item| format!("{site}:{}", item.len()))
            .collect())
    };
    let mut sealed = line();
    seal_journal_paths(&mut sealed, Some("site"), &seal);
    let text = sealed.to_string();
    assert!(!text.contains("Geheim"), "{text}");
    assert_eq!(sealed["root"], "site:18");
    assert_eq!(sealed["attempted"], 1);
    assert_eq!(sealed["withheld"], false);
    let failing = |_: &str, _: &[&str]| -> Result<Vec<String>, String> { Err("gesperrt".into()) };
    let mut withheld = line();
    seal_journal_paths(&mut withheld, Some("site"), &failing);
    assert!(!withheld.to_string().contains("Geheim"));
    assert_eq!(withheld["withheld"], true);
    let mut unknown = line();
    seal_journal_paths(&mut unknown, None, &seal);
    assert!(!unknown.to_string().contains("Geheim"));
}
