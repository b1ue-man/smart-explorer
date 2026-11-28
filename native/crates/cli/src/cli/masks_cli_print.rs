//! Text and JSON output of `se masks`. Neither form contains an entry value:
//! the results only carry `ListSummary`/`EntrySummary` (no value field).
use serde_json::Value;

pub(super) fn emit(value: &Value, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        );
    } else {
        print!("{}", text(value));
    }
}

fn flag(value: &Value, yes: &str, no: &str) -> String {
    if value.as_bool() == Some(true) {
        yes
    } else {
        no
    }
    .to_string()
}

fn list_line(list: &Value) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{} Einträge\t{} Schutzpfade\n",
        list["id"],
        list["name"].as_str().unwrap_or("?"),
        flag(&list["enabled"], "aktiv", "inaktiv"),
        flag(&list["locked"], "gesperrt", "offen"),
        list["entries"].as_array().map_or(0, Vec::len),
        list["scopes"].as_array().map_or(0, Vec::len),
    )
}

fn entry_line(entry: &Value) -> String {
    let tolerance = &entry["tolerance"];
    let tolerance = match tolerance["level"].as_str() {
        Some("custom") => format!("{}/{}", tolerance["trim"], tolerance["inner_tenths"]),
        Some(level) => level.to_string(),
        None => String::new(),
    };
    format!(
        "  {}\t{}\t{}\ttoleranz {}\t{}\t{}\n",
        entry["id"],
        entry["kind"].as_str().unwrap_or("?"),
        entry["label"].as_str().unwrap_or("?"),
        tolerance,
        flag(&entry["case_sensitive"], "groß/klein", "ohne groß/klein"),
        flag(&entry["whole_word"], "ganzes wort", "auch in wörtern"),
    )
}

fn hardening(value: &Value) -> String {
    if let Some(error) = value["error"].as_str() {
        return format!("härtung\tfehler\t{error}\n");
    }
    let mut out = String::new();
    if let Some(state) = value["state"].as_str() {
        out.push_str(&format!("härtung\t{state}"));
        if let Some(reason) = value["reason"].as_str() {
            out.push_str(&format!("\t{reason}"));
        }
        out.push('\n');
    }
    let hosts = if value.get("hosts").is_some() {
        &value["hosts"]
    } else {
        value
    };
    for host in ["claude", "codex"] {
        if let Some(state) = hosts[host]["state"].as_str() {
            out.push_str(&format!(
                "  {host}\t{state}\t{}\n",
                hosts[host]["path"].as_str().unwrap_or("")
            ));
        }
    }
    out
}

/// Human-readable form of one result of `masks_cli::execute`.
pub(super) fn text(value: &Value) -> String {
    let mut out = String::new();
    if let Some(lists) = value["lists"].as_array() {
        if lists.is_empty() {
            out.push_str("keine Maskenlisten\n");
        }
        for list in lists {
            out.push_str(&list_line(list));
        }
    }
    for key in ["list", "created"] {
        if value[key].is_object() {
            let list = &value[key];
            out.push_str(&list_line(list));
            for scope in list["scopes"].as_array().into_iter().flatten() {
                out.push_str(&format!(
                    "  schutzpfad\t{}\n",
                    scope.as_str().unwrap_or("?")
                ));
            }
            for entry in list["entries"].as_array().into_iter().flatten() {
                out.push_str(&entry_line(entry));
            }
        }
    }
    for entry in value["entries"].as_array().into_iter().flatten() {
        out.push_str(&entry_line(entry));
    }
    if value["entry"].is_object() {
        out.push_str(&entry_line(&value["entry"]));
    }
    for (key, label) in [
        ("added_scope", "schutzpfad hinzugefügt"),
        ("removed_scope", "schutzpfad entfernt"),
        ("removed_entry", "eintrag entfernt"),
        ("deleted", "liste gelöscht"),
    ] {
        if !value[key].is_null() {
            let shown = value[key]
                .as_str()
                .map_or_else(|| value[key].to_string(), str::to_owned);
            out.push_str(&format!("{label}\t{shown}\n"));
        }
    }
    if let Some(lockdown) = value["lockdown"].as_str() {
        out.push_str(&format!("agenten-sperre\t{lockdown}\n"));
    }
    if let Some(servers) = value["allowed_mcp_servers"].as_array() {
        let names: Vec<&str> = servers.iter().filter_map(Value::as_str).collect();
        out.push_str(&format!(
            "erlaubte mcp-server\t{}\n",
            if names.is_empty() {
                "keine".to_string()
            } else {
                names.join(", ")
            }
        ));
    }
    if !value["hardening"].is_null() {
        out.push_str(&hardening(&value["hardening"]));
    }
    out
}
