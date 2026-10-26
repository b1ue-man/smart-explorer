//! Format-preserving updates of our own Codex command/args, never remove/add.
#[cfg(test)]
#[path = "batches_codex_task_tests.rs"]
mod batches_codex_task_tests;
#[path = "codex_comments.rs"]
mod comments;
use super::{files, host_cli, managed};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use toml_edit::{value, Array, DocumentMut, Item, Table};
pub(super) const SERVER: &str = "smart-explorer";
const PASSED_ENV: &[&str] = &[
    "XDG_DATA_HOME",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
];
fn paths() -> Result<(PathBuf, PathBuf), String> {
    let home = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::support_dirs::home_dir().map(|h| h.join(".codex")))
        .ok_or("Codex home missing")?;
    Ok((
        home.join("config.toml"),
        home.join(".se-smart-explorer.v1.json"),
    ))
}
fn fields(table: &Item) -> Result<Value, String> {
    let command = table
        .get("command")
        .and_then(Item::as_str)
        .ok_or("integration_conflict: command missing or not a string")?;
    let args = table
        .get("args")
        .and_then(Item::as_array)
        .ok_or("integration_conflict: args missing or not an array")?;
    let args = args
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or("integration_conflict: non-string argument")
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"command":command,"args":args}))
}
fn document(bytes: Option<&[u8]>) -> Result<DocumentMut, String> {
    std::str::from_utf8(bytes.unwrap_or_default())
        .map_err(|e| e.to_string())?
        .parse::<DocumentMut>()
        .map_err(|e| format!("integration_conflict: invalid Codex TOML: {e}"))
}
fn owner_matches(table: &Item, owner: &Value) -> bool {
    fields(table)
        .ok()
        .and_then(|f| serde_json::to_vec(&f).ok())
        .is_some_and(|f| owner["entry_sha256"].as_str() == Some(managed::hash(&f).as_str()))
}
fn legacy_matches(table: &Item) -> bool {
    let Some(directory) = files::codex_skill_dir() else {
        return false;
    };
    let Some(marker) = files::read_marker(&directory) else {
        return false;
    };
    let Ok(Some(skill)) = managed::read(&directory.join("SKILL.md")) else {
        return false;
    };
    legacy_entry_matches(table, &marker, &skill)
}
fn legacy_entry_matches(table: &Item, marker: &Value, skill: &[u8]) -> bool {
    let Some(executable) = marker["executable"].as_str() else {
        return false;
    };
    marker.get("schema").is_none()
        && files::legacy_file_matches("SKILL.md", &skill, &marker)
        && fields(table).is_ok_and(|f| f == json!({"command":executable,"args":["mcp"]}))
}
fn legacy_managed_env(table: &Item) -> Value {
    // The released Windows installer did not pass any of these Unix values.
    if cfg!(windows) {
        return json!({});
    }
    let env = table.get("env").and_then(Item::as_table_like);
    Value::Object(
        PASSED_ENV
            .iter()
            .filter_map(|name| {
                env.and_then(|env| env.get(name))
                    .and_then(Item::as_str)
                    .filter(|value| !value.is_empty())
                    .map(|value| ((*name).into(), json!(value)))
            })
            .collect(),
    )
}
pub(super) fn install(executable: &Path) -> Result<Value, String> {
    if host_cli::find("codex").is_none() {
        // Keep the released marker until the registration itself can migrate.
        // Replacing only that marker would destroy its legacy ownership proof.
        let skill = match files::codex_skill_dir().filter(|path| path.join("SKILL.md").exists()) {
            Some(skill) => {
                preflight_skill_at(&skill)?;
                skill
            }
            None => files::write_codex_skill(executable)?,
        };
        return Ok(
            json!({"host":"codex","installed":false,"skill":skill,"error":"Codex (`codex`) was not found on PATH","next":"add the entry printed by se plugin config codex to the active Codex config.toml"}),
        );
    }
    configure(executable, false)
}
pub(super) fn refresh(executable: &Path) -> Result<Value, String> {
    configure(executable, true)
}
fn configure(executable: &Path, refresh: bool) -> Result<Value, String> {
    let (path, owner_path) = paths()?;
    preflight_skill()?;
    let _lock = managed::lock()?;
    let before = managed::read(&path)?;
    let owner_before = managed::read(&owner_path)?;
    let owner = owner_before
        .as_deref()
        .map(serde_json::from_slice::<Value>)
        .transpose()
        .map_err(|e| format!("integration_conflict: Codex ownership invalid: {e}"))?;
    let installation = files::installation_for_registration(executable)?;
    let installation_id = installation
        .as_ref()
        .map(|installation| installation.manifest.installation_id.as_str());
    if owner
        .as_ref()
        .and_then(|o| o["installation_id"].as_str())
        .is_some_and(|id| Some(id) != installation_id)
    {
        return Err("integration_conflict: Codex entry belongs to a different installation".into());
    }
    let mut doc = document(before.as_deref())?;
    if doc
        .get("mcp_servers")
        .is_some_and(|item| item.as_table_like().is_none())
    {
        return Err(format!(
            "integration_conflict: {} mcp_servers is not a table",
            path.display()
        ));
    }
    let existing = doc.get("mcp_servers").and_then(|t| t.get(SERVER));
    let existing_present = existing.is_some();
    let legacy_env = existing
        .filter(|table| legacy_matches(table))
        .map(legacy_managed_env);
    if let Some(table) = existing {
        if !owner
            .as_ref()
            .is_some_and(|owner| owner_matches(table, owner))
            && !legacy_matches(table)
        {
            return Err(format!(
                "integration_conflict: {} mcp_servers.{SERVER}.command/args",
                path.display()
            ));
        }
    } else if refresh {
        return Err(format!(
            "integration_conflict: {} managed entry was removed",
            path.display()
        ));
    }
    if !doc.contains_key("mcp_servers") {
        doc["mcp_servers"] = Item::Table(Table::new());
    }
    if doc["mcp_servers"].get(SERVER).is_none() {
        doc["mcp_servers"][SERVER] = Item::Table(Table::new());
    }
    let table = &mut doc["mcp_servers"][SERVER];
    // Update only the owned fields; unknown keys, env, tool filters, comments,
    // ordering and every unrelated table remain under DocumentMut's control.
    if table.get("command").and_then(Item::as_str) != Some(executable.to_string_lossy().as_ref()) {
        set_owned(
            table,
            "command",
            value(executable.to_string_lossy().as_ref()),
        );
    }
    if table.get("args").and_then(Item::as_array).is_none() {
        set_owned(table, "args", value(Array::from_iter(["mcp"])));
    }
    if !existing_present {
        let mut env = Table::new();
        if !cfg!(windows) {
            for name in PASSED_ENV {
                if let Ok(text) = std::env::var(name) {
                    if !text.is_empty() {
                        env.insert(name, value(text));
                    }
                }
            }
        }
        if !env.is_empty() {
            table["env"] = Item::Table(env);
        }
    }
    let entry = fields(table)?;
    let managed_env = owner
        .as_ref()
        .and_then(|o| o.get("managed_env"))
        .cloned()
        .unwrap_or_else(|| {
            if !existing_present {
                legacy_managed_env(table)
            } else {
                legacy_env.unwrap_or_else(|| json!({}))
            }
        });
    let ownership = json!({"schema":1,"installation_id":installation_id,"config":path,"managed_env":managed_env,"entry_sha256":managed::hash(&serde_json::to_vec(&entry).map_err(|e|e.to_string())?)});
    let after = doc.to_string().into_bytes();
    // Parse staged bytes before exposing any file.
    let verified = document(Some(&after))?;
    if fields(&verified["mcp_servers"][SERVER])? != entry {
        return Err("integration_registration_verification_failed".into());
    }
    if before.as_deref() != Some(after.as_slice()) {
        managed::replace(&path, before.as_deref(), &after)?;
    }
    let owner_bytes = serde_json::to_vec_pretty(&ownership).map_err(|e| e.to_string())?;
    if let Err(error) = managed::replace(&owner_path, owner_before.as_deref(), &owner_bytes) {
        if let Err(rollback) = restore_owned(&path, before.as_deref(), &after) {
            return Err(format!(
                "{error}; Codex configuration rollback failed: {rollback}"
            ));
        }
        return Err(error);
    }
    drop(_lock);
    let skill = match files::write_codex_skill(executable) {
        Ok(skill) => skill,
        Err(error) => {
            let _rollback_lock = managed::lock()?;
            let config_rollback = restore_owned(&path, before.as_deref(), &after);
            let owner_rollback = restore_owned(&owner_path, owner_before.as_deref(), &owner_bytes);
            let failures = [config_rollback, owner_rollback]
                .into_iter()
                .filter_map(Result::err)
                .collect::<Vec<_>>();
            if !failures.is_empty() {
                return Err(format!(
                    "{error}; Codex registration rollback failed: {}",
                    failures.join("; ")
                ));
            }
            return Err(error);
        }
    };
    let cli = host_cli::find("codex");
    let host_verified = cli
        .as_ref()
        .map(|cli| host_cli::run(cli, &["mcp", "get", SERVER, "--json"]).is_ok_and(|o| o.success));
    Ok(
        json!({"host":"codex","installed":true,"verified":host_verified,"cli":cli,"config":path,"skill":skill,"next":"Restart the Codex MCP server in the app, restart the IDE extension, or start a new CLI process to initialize the new server."}),
    )
}
pub(super) fn uninstall() -> Result<Value, String> {
    let (path, owner_path) = paths()?;
    if let Some(skill) = files::codex_skill_dir() {
        if !files::generated_files_absent(&skill)? {
            files::verify_owned(&skill)?;
        }
    }
    let _lock = managed::lock()?;
    let before = managed::read(&path)?;
    let owner_bytes = managed::read(&owner_path)?;
    let owner = owner_bytes
        .as_deref()
        .map(serde_json::from_slice::<Value>)
        .transpose()
        .map_err(|e| format!("integration_conflict: Codex ownership invalid: {e}"))?;
    let mut doc = document(before.as_deref())?;
    if doc
        .get("mcp_servers")
        .is_some_and(|item| item.as_table_like().is_none())
    {
        return Err(format!(
            "integration_conflict: {} mcp_servers is not a table",
            path.display()
        ));
    }
    if let Some(table) = doc.get("mcp_servers").and_then(|t| t.get(SERVER)) {
        let legacy = owner.is_none() && legacy_matches(table);
        verify_removable_entry(table, owner.as_ref(), legacy)
            .map_err(|error| format!("{error} ({})", path.display()))?;
        comments::remove_server(&mut doc)?;
        managed::replace(&path, before.as_deref(), doc.to_string().as_bytes())?;
    }
    if owner_bytes.is_some() {
        std::fs::remove_file(owner_path).map_err(|e| e.to_string())?;
    }
    drop(_lock);
    let removed = files::codex_skill_dir()
        .map(|d| files::remove_generated(&d))
        .transpose()?
        .unwrap_or(false);
    Ok(json!({"host":"codex","removed_skill":removed,"installed":false}))
}
fn verify_removable_entry(table: &Item, owner: Option<&Value>, legacy: bool) -> Result<(), String> {
    if !owner.is_some_and(|owner| owner_matches(table, owner)) && !legacy {
        return Err("integration_conflict: command/args ownership differs".into());
    }
    let keys = table
        .as_table_like()
        .ok_or("integration_conflict: server is not a table")?
        .iter()
        .map(|(key, _)| key);
    if keys
        .into_iter()
        .any(|key| !matches!(key, "command" | "args" | "env"))
    {
        return Err("integration_conflict: contains user settings; entry retained".into());
    }
    let legacy_env = legacy_managed_env(table);
    let managed_env = match owner {
        Some(owner) => owner
            .get("managed_env")
            .ok_or("integration_conflict: managed env record missing")?,
        None => &legacy_env,
    };
    let expected = managed_env
        .as_object()
        .ok_or("integration_conflict: managed env record missing")?;
    if let Some(env) = table.get("env") {
        let env = env
            .as_table_like()
            .ok_or("integration_conflict: env is not a table")?;
        if env.len() != expected.len()
            || env
                .iter()
                .any(|(key, value)| value.as_str() != expected.get(key).and_then(Value::as_str))
        {
            return Err(
                "integration_conflict: contains user environment settings; entry retained".into(),
            );
        }
    } else if !expected.is_empty() {
        return Err("integration_conflict: managed environment removed; entry retained".into());
    }
    Ok(())
}
pub(super) fn status(executable: &Path) -> Value {
    let configured = (|| -> Result<Value, String> {
        let (path, owner_path) = paths()?;
        let bytes = managed::read(&path)?;
        let doc = document(bytes.as_deref())?;
        let owner = managed::read(&owner_path)?
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        let table = doc
            .get("mcp_servers")
            .and_then(|servers| servers.get(SERVER));
        let installation = crate::mcp_launcher::Installation::for_image(executable)?;
        let current = table.zip(owner.as_ref()).is_some_and(|(table, owner)| {
            owner["schema"] == 1
                && owner_matches(table, owner)
                && fields(table).ok().is_some_and(|fields| {
                    fields["args"] == json!(["mcp"])
                        && fields["command"].as_str().is_some_and(|command| {
                            if let Some(installation) = &installation {
                                owner["installation_id"].as_str()
                                    == Some(installation.manifest.installation_id.as_str())
                                    && installation
                                        .registered_launcher_allowed(Path::new(command))
                                        .unwrap_or(false)
                            } else {
                                owner["installation_id"].is_null()
                                    && Path::new(command) == executable
                            }
                        })
                })
        });
        let conflict = table.is_some_and(|table| {
            !owner
                .as_ref()
                .is_some_and(|owner| owner_matches(table, owner))
                && !legacy_matches(table)
        });
        Ok(
            json!({"present":table.is_some(), "current":current,"mode":if installation.is_some() {"managed"}else{"legacy_cli"},
            "integration_conflict":conflict, "path":path}),
        )
    })();
    json!({"host":"codex", "cli":host_cli::find("codex"),
        "server":configured.unwrap_or_else(|error| json!({"error":error})),
        "skill":files::generated_state(files::codex_skill_dir(),executable)})
}

// Call while holding the integration lock. A concurrent editor's newer bytes
// are retained; first registration must restore absence as well as old content.
fn restore_owned(path: &Path, before: Option<&[u8]>, after: &[u8]) -> Result<(), String> {
    if managed::read(path)?.as_deref() != Some(after) {
        return Ok(());
    }
    if let Some(before) = before {
        managed::replace(path, Some(after), before)
    } else {
        std::fs::remove_file(path).map_err(|error| format!("{}: {error}", path.display()))
    }
}

fn set_owned(table: &mut Item, key: &str, mut replacement: Item) {
    if let (Some(before), Some(after)) = (
        table.get(key).and_then(Item::as_value),
        replacement.as_value_mut(),
    ) {
        *after.decor_mut() = before.decor().clone();
    }
    table[key] = replacement;
}
fn preflight_skill() -> Result<(), String> {
    let directory = files::codex_skill_dir().ok_or("Codex home missing")?;
    preflight_skill_at(&directory)
}
fn preflight_skill_at(directory: &Path) -> Result<(), String> {
    let Some(bytes) = managed::read(&directory.join("SKILL.md"))? else {
        return Ok(());
    };
    let marker = files::read_marker(&directory)
        .ok_or("integration_conflict: Codex skill ownership missing")?;
    if marker.get("schema").is_none() && files::legacy_file_matches("SKILL.md", &bytes, &marker) {
        return Ok(());
    }
    files::verify_owned(directory)
}
