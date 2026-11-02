//! Stage `vault_mcp` (vault plan VLT-M9, C7.3, U26-13): the vault functions
//! of `se mcp` through the built `se`, in both MCP lifecycles. The tool `se`
//! runs every `se vault` command, the file tools take `vault://` targets,
//! and `status` counts the vaults without naming one.
use serde_json::{json, Value};

use super::vault_task_client::{Lifecycle, McpClient};
use crate::cli::vault::task_env::{assert_keys, field, vault_url, TaskEnv, PASSWORD};

/// Whether the list of the tools names `name`.
fn has_tool(tools: &Value, name: &str) -> bool {
    tools["tools"]
        .as_array()
        .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == name))
}

/// The state `se vault list --json` shows for `site`.
fn state_of(listed: &Value, site: &str) -> String {
    let sites = listed["vaults"]
        .as_array()
        .expect("vaults ist eine Liste")
        .iter()
        .flat_map(|group| group["sites"].as_array().expect("sites").iter());
    for entry in sites {
        if entry["site"] == site {
            return field(entry, "state");
        }
    }
    panic!("{site} fehlt in {listed}");
}

/// The vault tools of the server in one lifecycle: list, unlock with the
/// password on standard input (argument `stdin`), write and read files of
/// the vault, `status`, lock, and the words of a locked vault.
fn vault_tools_in(lifecycle: Lifecycle) {
    let env = TaskEnv::isolated();
    let site = env.create_vault("orte/mcp", "MCP-Tresor");
    let mut client = McpClient::start(&env, lifecycle);

    // The opening names the server; its instructions and the description of
    // the tool `se` speak of vaults.
    let handshake = client.handshake().clone();
    match lifecycle {
        Lifecycle::Classic => {
            assert_eq!(handshake["protocolVersion"], "2025-06-18", "{handshake}");
            assert_eq!(
                handshake["serverInfo"]["name"], "smart-explorer",
                "{handshake}"
            );
        }
        Lifecycle::Stateless => {
            let versions = handshake["supportedVersions"]
                .as_array()
                .expect("supportedVersions ist eine Liste");
            assert!(
                versions.iter().any(|version| version == "2026-07-28"),
                "{handshake}"
            );
            assert_eq!(
                handshake["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "smart-explorer",
                "{handshake}"
            );
        }
    }
    assert!(
        handshake["instructions"]
            .as_str()
            .is_some_and(|text| text.contains("vault://<site>/path")),
        "{handshake}"
    );
    let tools = client.request("tools/list", Value::Null);
    for name in ["se", "status", "list", "read", "write"] {
        assert!(has_tool(&tools, name), "Werkzeug {name} fehlt: {tools}");
    }
    let se_tool = tools["tools"]
        .as_array()
        .expect("tools ist eine Liste")
        .iter()
        .find(|tool| tool["name"] == "se")
        .expect("Werkzeug se");
    assert!(
        se_tool["description"]
            .as_str()
            .is_some_and(|text| text.contains("\"vault\",\"list\",\"--json\"")),
        "{se_tool}"
    );

    // The vault is listed and locked.
    let listed = client.se(&["vault", "list", "--json"], None);
    assert_eq!(listed.code, 0, "{listed}");
    assert_eq!(state_of(&listed.json(), &site), "locked");

    // Unlocking: the password comes as `stdin`, never as an argument.
    let unlocked = client.se(
        &[
            "vault",
            "unlock",
            site.as_str(),
            "--password-stdin",
            "--json",
        ],
        Some(PASSWORD),
    );
    assert_eq!(unlocked.code, 0, "{unlocked}");
    assert_eq!(unlocked.json()["state"], "unlocked");
    let listed = client.se(&["vault", "list", "--json"], None);
    assert_eq!(state_of(&listed.json(), &site), "unlocked");

    // The file tools and `se cat` reach the unlocked vault.
    let target = vault_url(&site, "/hallo.txt");
    let written = client.call(
        "write",
        json!({ "target": target, "content": "Hallo Tresor\n" }),
    );
    assert!(!written.is_error, "{}", written.data);
    let cat = client.se(&["cat", target.as_str()], None);
    assert_eq!(cat.code, 0, "{cat}");
    assert_eq!(cat.stdout, "Hallo Tresor\n");
    let read = client.call("read", json!({ "target": target }));
    assert!(!read.is_error, "{}", read.data);
    assert_eq!(read.data["content"], "Hallo Tresor\n");
    assert_eq!(read.data["size"], 13);
    let listing = client.call("list", json!({ "target": vault_url(&site, "/") }));
    assert!(!listing.is_error, "{}", listing.data);
    let entries = listing.data["entries"]
        .as_array()
        .expect("entries ist eine Liste");
    assert!(
        entries
            .iter()
            .any(|entry| entry["name"] == "hallo.txt" && entry["size"] == 13),
        "{}",
        listing.data
    );

    // `status` counts the vaults.
    let status = client.call("status", json!({}));
    assert!(!status.is_error, "{}", status.data);
    assert_eq!(status.data["vaults"]["entered"], 1, "{}", status.data);
    assert_eq!(status.data["vaults"]["unlocked"], 1, "{}", status.data);

    // Locking: `locked <site>`, and a locked vault names the unlock call, as
    // an `se` call and as a file tool.
    let locked = client.se(&["vault", "lock", "--all"], None);
    assert_eq!(locked.code, 0, "{locked}");
    assert!(locked.stdout.contains(&site), "{locked}");
    let unlock = format!("se vault unlock {site}");
    let cat = client.se(&["cat", target.as_str()], None);
    assert_eq!(cat.code, 1, "{cat}");
    assert!(cat.stderr.contains(&unlock), "{cat}");
    let read = client.call("read", json!({ "target": target }));
    assert!(read.is_error, "{}", read.data);
    assert!(
        read.data["error"]
            .as_str()
            .is_some_and(|text| text.contains(&unlock)),
        "{}",
        read.data
    );
    let listed = client.se(&["vault", "list", "--json"], None);
    assert_eq!(state_of(&listed.json(), &site), "locked");
    client.finish();
}

#[test]
#[ignore = "braucht die gebaute se in SMART_EXPLORER_VAULT_SE_BIN (Stufe vault-mcp, env=SMART_EXPLORER_VAULT_SE_BIN={se})"]
fn vault_task_mcp_both_lifecycles() {
    vault_tools_in(Lifecycle::Classic);
    vault_tools_in(Lifecycle::Stateless);
}

#[test]
#[ignore = "braucht die gebaute se in SMART_EXPLORER_VAULT_SE_BIN (Stufe vault-mcp, env=SMART_EXPLORER_VAULT_SE_BIN={se})"]
fn vault_task_mcp_status_without_paths() {
    let env = TaskEnv::isolated();
    // Two vaults at places with telling names, a file and a folder with
    // telling names inside the unlocked one, and a vault job with its copy.
    let alpha = env.create_vault("orte/geheimer-ort-alpha/tresor", "Geheimlabel-Alpha");
    env.create_vault("orte/geheimer-ort-beta/tresor", "Geheimlabel-Beta");
    let pair = env.sync_pair(&alpha);
    env.unlock(&alpha);
    let local = env.path("input/inhalt.txt");
    std::fs::write(&local, "Inhalt\n").expect("lokale Datei");
    env.se(&["mkdir", vault_url(&alpha, "/Geheimordner-Gamma").as_str()])
        .ok();
    env.put(&local, &alpha, "/Geheimordner-Gamma/Geheimdatei-Delta.txt");
    let replica = env
        .se(&["vault", "show", pair.site_b.as_str(), "--json"])
        .ok()
        .json();
    let secrets = [
        "Geheimlabel-Alpha".to_string(),
        "Geheimlabel-Beta".to_string(),
        "geheimer-ort-alpha".to_string(),
        "geheimer-ort-beta".to_string(),
        "Geheimordner-Gamma".to_string(),
        "Geheimdatei-Delta".to_string(),
        "Tresor Geheimlabel-Alpha".to_string(),
        field(&replica, "location"),
    ];

    let mut client = McpClient::start(&env, Lifecycle::Classic);
    let keys = ["entered", "unlocked", "locked", "jobs", "full_list"];
    let status = client.call("status", json!({}));
    assert!(!status.is_error, "{}", status.data);
    let vaults = &status.data["vaults"];
    assert_keys(vaults, &keys, "status: vaults");
    // Three places: the two vaults and the copy the job keeps.
    assert_eq!(vaults["entered"], 3, "{vaults}");
    assert_eq!(vaults["unlocked"], 1, "{vaults}");
    assert_eq!(vaults["locked"], 2, "{vaults}");
    assert_eq!(vaults["jobs"], 1, "{vaults}");
    let full_list = field(vaults, "full_list");
    assert!(
        full_list.contains("vault") && full_list.contains("list") && !full_list.contains('/'),
        "{full_list}"
    );
    // The whole answer of `status` names no place, label, job or cleartext
    // name of a vault.
    let text = status.data.to_string();
    for secret in &secrets {
        assert!(!text.contains(secret.as_str()), "{secret} in {text}");
    }
    // The list, in contrast, names them: the test would not notice a leak
    // if the places and names were not there to be found.
    let listed = client.se(&["vault", "list", "--json"], None);
    assert_eq!(listed.code, 0, "{listed}");
    for secret in &secrets[..3] {
        assert!(
            listed.stdout.contains(secret.as_str()),
            "{secret}: {listed}"
        );
    }

    // The numbers follow the locks.
    let locked = client.se(&["vault", "lock", "--all"], None);
    assert_eq!(locked.code, 0, "{locked}");
    let status = client.call("status", json!({}));
    let vaults = &status.data["vaults"];
    assert_keys(vaults, &keys, "status: vaults (gesperrt)");
    assert_eq!(vaults["unlocked"], 0, "{vaults}");
    assert_eq!(vaults["locked"], 3, "{vaults}");
    client.finish();
}
