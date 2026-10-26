//! Format-preserving updates of our own Codex command/args, never remove/add.
#[cfg(test)]
#[path = "batches_codex_task_tests.rs"]
mod batches_codex_task_tests;
use super::{files,host_cli,managed};
use serde_json::{json,Value};
use std::path::{Path,PathBuf};
use toml_edit::{DocumentMut,Item,value,Array,Table};
pub(super) const SERVER:&str="smart-explorer";
const PASSED_ENV:&[&str]=&["XDG_DATA_HOME","XDG_RUNTIME_DIR","DBUS_SESSION_BUS_ADDRESS"];
fn paths()->Result<(PathBuf,PathBuf),String> {
    let home=std::env::var_os("CODEX_HOME").filter(|value|!value.is_empty()).map(PathBuf::from).or_else(||crate::support_dirs::home_dir().map(|h|h.join(".codex"))).ok_or("Codex home missing")?;
    Ok((home.join("config.toml"),home.join(".se-smart-explorer.v1.json")))
}
fn fields(table:&Item)->Result<Value,String> {
    let command=table.get("command").and_then(Item::as_str).ok_or("integration_conflict: command missing or not a string")?;
    let args=table.get("args").and_then(Item::as_array).ok_or("integration_conflict: args missing or not an array")?;
    let args=args.iter().map(|v|v.as_str().map(str::to_owned).ok_or("integration_conflict: non-string argument")).collect::<Result<Vec<_>,_>>()?;
    Ok(json!({"command":command,"args":args}))
}
fn document(bytes:Option<&[u8]>)->Result<DocumentMut,String> {
    std::str::from_utf8(bytes.unwrap_or_default()).map_err(|e|e.to_string())?.parse::<DocumentMut>().map_err(|e|format!("integration_conflict: invalid Codex TOML: {e}"))
}
fn owner_matches(table:&Item,owner:&Value)->bool {
    fields(table).ok().and_then(|f|serde_json::to_vec(&f).ok()).is_some_and(|f|owner["entry_sha256"].as_str()==Some(managed::hash(&f).as_str()))
}
fn legacy_matches(table:&Item)->bool {
    let Some(directory)=files::codex_skill_dir() else {return false;};
    let Some(marker)=files::read_marker(&directory) else {return false;};
    let Some(executable)=marker["executable"].as_str() else {return false;};
    let Ok(Some(skill))=managed::read(&directory.join("SKILL.md")) else {return false;};
    marker.get("schema").is_none() && files::legacy_file_matches("SKILL.md",&skill,&marker)
        && fields(table).is_ok_and(|f|f==json!({"command":executable,"args":["mcp"]}))
}
pub(super) fn install(executable:&Path)->Result<Value,String> {
    if host_cli::find("codex").is_none() {
        let skill=files::write_codex_skill(executable)?;
        return Ok(json!({"host":"codex","installed":false,"skill":skill,"error":"Codex (`codex`) was not found on PATH","next":"add the entry printed by se plugin config codex to the active Codex config.toml"}));
    }
    configure(executable,false)
}
pub(super) fn refresh(executable:&Path)->Result<Value,String> {configure(executable,true)}
fn configure(executable:&Path,refresh:bool)->Result<Value,String> {
    let (path,owner_path)=paths()?;
    preflight_skill()?;
    let _lock=managed::lock()?;
    let before=managed::read(&path)?;
    let owner_before=managed::read(&owner_path)?;
    let owner=owner_before.as_deref().and_then(|b|serde_json::from_slice::<Value>(b).ok());
    let installation=crate::mcp_launcher::Installation::for_image(executable)?.ok_or("integration_installation_missing")?;
    if owner.as_ref().and_then(|o|o["installation_id"].as_str()).is_some_and(|id|id!=installation.manifest.installation_id) {
        return Err("integration_conflict: Codex entry belongs to a different installation".into());
    }
    let mut doc=document(before.as_deref())?;
    if doc.get("mcp_servers").is_some_and(|item|item.as_table_like().is_none()) {return Err(format!("integration_conflict: {} mcp_servers is not a table",path.display()));}
    let existing=doc.get("mcp_servers").and_then(|t|t.get(SERVER));
    let existing_present=existing.is_some();
    if let Some(table)=existing {
        if !owner.as_ref().is_some_and(|owner|owner_matches(table,owner)) && !legacy_matches(table) {
            return Err(format!("integration_conflict: {} mcp_servers.{SERVER}.command/args",path.display()));
        }
    } else if refresh {return Err(format!("integration_conflict: {} managed entry was removed",path.display()));}
    if !doc.contains_key("mcp_servers") {doc["mcp_servers"]=Item::Table(Table::new());}
    if doc["mcp_servers"].get(SERVER).is_none() {doc["mcp_servers"][SERVER]=Item::Table(Table::new());}
    let table=&mut doc["mcp_servers"][SERVER];
    // Update only the owned fields; unknown keys, env, tool filters, comments,
    // ordering and every unrelated table remain under DocumentMut's control.
    if table.get("command").and_then(Item::as_str)!=Some(executable.to_string_lossy().as_ref()) {set_owned(table,"command",value(executable.to_string_lossy().as_ref()));}
    if table.get("args").and_then(Item::as_array).is_none() {set_owned(table,"args",value(Array::from_iter(["mcp"])));}
    if !existing_present {
        let mut env=Table::new();
        for name in PASSED_ENV {if let Ok(text)=std::env::var(name) {if !text.is_empty() {env.insert(name,value(text));}}}
        if !env.is_empty() {table["env"]=Item::Table(env);}
    }
    let entry=fields(table)?;
    let managed_env=owner.as_ref().and_then(|o|o.get("managed_env")).cloned().unwrap_or_else(|| {
        if !existing_present { let env=PASSED_ENV.iter().filter_map(|name|std::env::var(name).ok().filter(|v|!v.is_empty()).map(|v|((*name).to_owned(),json!(v)))).collect::<serde_json::Map<_,_>>(); Value::Object(env) } else {json!({})}
    });
    let ownership=json!({"schema":1,"installation_id":installation.manifest.installation_id,"config":path,"managed_env":managed_env,"entry_sha256":managed::hash(&serde_json::to_vec(&entry).map_err(|e|e.to_string())?)});
    let after=doc.to_string().into_bytes();
    // Parse staged bytes before exposing any file.
    let verified=document(Some(&after))?;
    if fields(&verified["mcp_servers"][SERVER])?!=entry {return Err("integration_registration_verification_failed".into());}
    if before.as_deref()!=Some(after.as_slice()) {managed::replace(&path,before.as_deref(),&after)?;}
    let owner_bytes=serde_json::to_vec_pretty(&ownership).map_err(|e|e.to_string())?;
    if let Err(error)=managed::replace(&owner_path,owner_before.as_deref(),&owner_bytes) {
        if let Err(rollback)=restore_owned(&path,before.as_deref(),&after) {
            return Err(format!("{error}; Codex configuration rollback failed: {rollback}"));
        }
        return Err(error);
    }
    drop(_lock);
    let skill=match files::write_codex_skill(executable) {
        Ok(skill)=>skill,
        Err(error)=>{
            let _rollback_lock=managed::lock()?;
            let config_rollback=restore_owned(&path,before.as_deref(),&after);
            let owner_rollback=restore_owned(&owner_path,owner_before.as_deref(),&owner_bytes);
            let failures=[config_rollback,owner_rollback].into_iter().filter_map(Result::err).collect::<Vec<_>>();
            if !failures.is_empty() {
                return Err(format!("{error}; Codex registration rollback failed: {}",failures.join("; ")));
            }
            return Err(error);
        }
    };
    let cli=host_cli::find("codex");
    let host_verified=cli.as_ref().map(|cli|host_cli::run(cli,&["mcp","get",SERVER,"--json"]).is_ok_and(|o|o.success));
    Ok(json!({"host":"codex","installed":true,"verified":host_verified,"cli":cli,"config":path,"skill":skill,"next":"Restart the Codex MCP server in the app, restart the IDE extension, or start a new CLI process to initialize the new server."}))
}
pub(super) fn uninstall()->Result<Value,String> {
    let (path,owner_path)=paths()?;
    if let Some(skill)=files::codex_skill_dir() {files::verify_owned(&skill)?;}
    let _lock=managed::lock()?;
    let Some(before)=managed::read(&path)? else {return Ok(json!({"host":"codex","installed":false}));};
    let owner=managed::read(&owner_path)?.and_then(|b|serde_json::from_slice::<Value>(&b).ok()).ok_or("integration_conflict: Codex ownership fingerprint missing")?;
    let mut doc=document(Some(&before))?;
    if let Some(table)=doc.get("mcp_servers").and_then(|t|t.get(SERVER)) {
        if !owner_matches(table,&owner) {return Err(format!("integration_conflict: {} command/args differ",path.display()));}
        let keys=table.as_table_like().ok_or("integration_conflict: server is not a table")?.iter().map(|(k,_)|k).collect::<Vec<_>>();
        if keys.iter().any(|k|!matches!(*k,"command"|"args"|"env")) {
            return Err(format!("integration_conflict: {} contains user settings; entry retained",path.display()));
        }
        if let Some(env)=table.get("env") {
            let expected=owner["managed_env"].as_object().ok_or("integration_conflict: managed env record missing")?;
            let env=env.as_table_like().ok_or("integration_conflict: env is not a table")?;
            if env.len()!=expected.len() || env.iter().any(|(key,value)|value.as_str()!=expected.get(key).and_then(Value::as_str)) {return Err(format!("integration_conflict: {} contains user environment settings; entry retained",path.display()));}
        }
        doc["mcp_servers"].as_table_like_mut().ok_or("integration_conflict: mcp_servers is not a table")?.remove(SERVER);
        managed::replace(&path,Some(&before),doc.to_string().as_bytes())?;
    }
    std::fs::remove_file(owner_path).map_err(|e|e.to_string())?;
    drop(_lock);
    let removed=files::codex_skill_dir().map(|d|files::remove_generated(&d)).transpose()?.unwrap_or(false);
    Ok(json!({"host":"codex","removed_skill":removed,"installed":false}))
}
pub(super) fn status(executable: &Path) -> Value {
    let configured = (|| -> Result<Value, String> {
        let (path, owner_path) = paths()?;
        let bytes = managed::read(&path)?;
        let doc = document(bytes.as_deref())?;
        let owner = managed::read(&owner_path)?
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        let table = doc.get("mcp_servers").and_then(|servers| servers.get(SERVER));
        let installation = crate::mcp_launcher::Installation::for_image(executable)?;
        let current = table.zip(owner.as_ref()).zip(installation.as_ref())
            .is_some_and(|((table, owner), installation)| {
                owner["schema"] == 1
                    && owner["installation_id"].as_str()
                        == Some(installation.manifest.installation_id.as_str())
                    && owner_matches(table, owner)
                    && fields(table).ok().is_some_and(|fields| {
                        fields["args"] == json!(["mcp"])
                            && fields["command"].as_str().is_some_and(|command| {
                                installation.registered_launcher_allowed(Path::new(command))
                                    .unwrap_or(false)
                            })
                    })
            });
        let conflict = table.is_some_and(|table| {
            !owner.as_ref().is_some_and(|owner| owner_matches(table, owner))
                && !legacy_matches(table)
        });
        Ok(json!({"present":table.is_some(), "current":current,
            "integration_conflict":conflict, "path":path}))
    })();
    json!({"host":"codex", "cli":host_cli::find("codex"),
        "server":configured.unwrap_or_else(|error| json!({"error":error})),
        "skill":files::generated_state(files::codex_skill_dir(),executable)})
}

// Call while holding the integration lock. A concurrent editor's newer bytes
// are retained; first registration must restore absence as well as old content.
fn restore_owned(path:&Path,before:Option<&[u8]>,after:&[u8])->Result<(),String> {
    if managed::read(path)?.as_deref()!=Some(after) {return Ok(());}
    if let Some(before)=before {managed::replace(path,Some(after),before)}
    else {std::fs::remove_file(path).map_err(|error|format!("{}: {error}",path.display()))}
}

fn set_owned(table:&mut Item,key:&str,mut replacement:Item) {
    if let (Some(before),Some(after))=(table.get(key).and_then(Item::as_value),replacement.as_value_mut()) {
        *after.decor_mut()=before.decor().clone();
    }
    table[key]=replacement;
}
fn preflight_skill()->Result<(),String> {
    let directory=files::codex_skill_dir().ok_or("Codex home missing")?;
    let Some(bytes)=managed::read(&directory.join("SKILL.md"))? else {return Ok(());};
    let marker=files::read_marker(&directory).ok_or("integration_conflict: Codex skill ownership missing")?;
    if marker.get("schema").is_none() && files::legacy_file_matches("SKILL.md",&bytes,&marker) {return Ok(());}
    files::verify_owned(&directory)
}
