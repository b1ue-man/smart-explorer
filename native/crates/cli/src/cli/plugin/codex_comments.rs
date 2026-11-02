//! Comments belong to the user even when their parsed table belongs to se.
use toml_edit::{Decor, DocumentMut, Item, Key, RawString, TableLike, Value};

pub(super) fn remove_server(doc: &mut DocumentMut) -> Result<(), String> {
    let heading = leading_comments(&doc.to_string());
    let servers = doc.get_mut("mcp_servers")
        .and_then(Item::as_table_like_mut)
        .ok_or("integration_conflict: mcp_servers is not a table")?;
    let mut retained = String::new();
    if let Some((key, item)) = servers.get_key_value(super::SERVER) {
        key_comments(key, &mut retained)?;
        item_comments(item, &mut retained)?;
    }
    servers.remove(super::SERVER);
    if !retained.is_empty() {
        // Removed inline/header comments become standalone comments. Surviving
        // tables, key/value bytes and their own decorations are never rewritten.
        if !heading.is_empty() && leading_comments(&doc.to_string()) != heading {
            // The file heading can be attached to the first removed header.
            // Keep that heading first; move only the remaining removed notes.
            let Some(tail) = retained.strip_prefix(&heading) else {
                return Err("integration_conflict: Codex heading preservation failed".into());
            };
            let tail = tail.to_owned();
            let prefix = doc.decor().prefix().and_then(RawString::as_str).unwrap_or_default();
            let prefix = format!("{heading}{prefix}");
            doc.decor_mut().set_prefix(prefix);
            retained = tail;
        }
        if !retained.is_empty() {
            let trailing = doc.trailing().as_str()
                .ok_or("integration_conflict: unresolved Codex trailing span")?;
            let trailing = format!("{trailing}\n{retained}");
            doc.set_trailing(trailing);
        }
    }
    Ok(())
}

fn leading_comments(text: &str) -> String {
    text.split_inclusive('\n')
        .take_while(|line| line.trim().is_empty() || line.trim_start().starts_with('#'))
        .filter(|line| line.contains('#'))
        .collect()
}

fn raw_comments(raw: Option<&RawString>, retained: &mut String) -> Result<(), String> {
    let Some(raw) = raw else { return Ok(()); };
    let text = raw.as_str().ok_or("integration_conflict: unresolved Codex comment span")?;
    // Decorations contain whitespace/comments only: hashes inside TOML strings
    // are value representations and are never visited here.
    for line in text.split_inclusive('\n') {
        if line.contains('#') {
            retained.push_str(line);
            if !line.ends_with('\n') {
                retained.push('\n');
            }
        }
    }
    Ok(())
}

fn decor_comments(decor: &Decor, retained: &mut String) -> Result<(), String> {
    raw_comments(decor.prefix(), retained)?;
    raw_comments(decor.suffix(), retained)
}

fn key_comments(key: &Key, retained: &mut String) -> Result<(), String> {
    decor_comments(key.leaf_decor(), retained)?;
    decor_comments(key.dotted_decor(), retained)
}

fn table_comments(table: &dyn TableLike, retained: &mut String) -> Result<(), String> {
    for (name, item) in table.iter() {
        if let Some(key) = table.key(name) {
            key_comments(key, retained)?;
        }
        item_comments(item, retained)?;
    }
    Ok(())
}

fn item_comments(item: &Item, retained: &mut String) -> Result<(), String> {
    match item {
        Item::Table(table) => {
            decor_comments(table.decor(), retained)?;
            table_comments(table, retained)
        }
        Item::Value(value) => value_comments(value, retained),
        Item::ArrayOfTables(tables) => {
            for table in tables.iter() {
                decor_comments(table.decor(), retained)?;
                table_comments(table, retained)?;
            }
            Ok(())
        }
        Item::None => Ok(()),
    }
}

fn value_comments(value: &Value, retained: &mut String) -> Result<(), String> {
    decor_comments(value.decor(), retained)?;
    match value {
        Value::Array(array) => {
            for value in array.iter() {
                value_comments(value, retained)?;
            }
            raw_comments(Some(array.trailing()), retained)
        }
        Value::InlineTable(table) => {
            table_comments(table, retained)?;
            raw_comments(Some(table.trailing()), retained)
        }
        _ => Ok(()),
    }
}
