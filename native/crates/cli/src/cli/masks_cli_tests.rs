//! `se masks`: command line, values from stdin, output without values.
use super::{print, read_values, store, MasksArgs, MasksCommand};
use clap::Parser;
use serde_json::json;

#[derive(Parser)]
struct Cli {
    #[command(flatten)]
    args: MasksArgs,
}

fn parse(args: &[&str]) -> MasksCommand {
    let mut all = vec!["se-masks"];
    all.extend_from_slice(args);
    Cli::try_parse_from(all).unwrap().args.command
}

#[test]
fn masks_lock_task_cli_commands_parse() {
    assert!(matches!(
        parse(&["list", "--json"]),
        MasksCommand::List { json: true }
    ));
    match parse(&[
        "add",
        "Kunden",
        "--kind",
        "fuzzy",
        "--tolerance",
        "2/1.5",
        "--label",
        "Name",
    ]) {
        MasksCommand::Add {
            list,
            kind,
            options,
            whole_input,
            json,
        } => {
            assert_eq!(
                (list.as_str(), kind.as_str(), whole_input, json),
                ("Kunden", "fuzzy", false, false)
            );
            assert_eq!(options.label.as_deref(), Some("Name"));
            assert_eq!(options.tolerance.as_deref(), Some("2/1.5"));
        }
        _ => panic!("add"),
    }
    match parse(&[
        "update",
        "1",
        "7",
        "--case-sensitive",
        "true",
        "--value-stdin",
    ]) {
        MasksCommand::Update {
            entry,
            options,
            value_stdin,
            ..
        } => {
            assert_eq!(entry, 7);
            assert_eq!(options.case_sensitive, Some(true));
            assert!(value_stdin);
        }
        _ => panic!("update"),
    }
    assert!(matches!(
        parse(&["scope", "add", "1", "@nas:/p"]),
        MasksCommand::Scope { .. }
    ));
    assert!(matches!(
        parse(&["servers", "allow", "memory"]),
        MasksCommand::Servers { .. }
    ));
    for command in ["lock", "unlock", "enable", "disable", "delete"] {
        let _ = parse(&[command, "Kunden"]);
    }
    assert!(Cli::try_parse_from(["se-masks", "show"]).is_err());
}

#[test]
fn masks_lock_task_cli_values_from_stdin() {
    let mut lines: &[u8] = b"Alpha\r\n\nBeta Gamma\n";
    assert_eq!(
        read_values(&mut lines, false).unwrap(),
        vec!["Alpha", "Beta Gamma"]
    );
    let mut whole: &[u8] = b"a\\d+\nb\n";
    assert_eq!(read_values(&mut whole, true).unwrap(), vec!["a\\d+\nb"]);
    let mut empty: &[u8] = b"\n";
    assert!(read_values(&mut empty, true).unwrap().is_empty());
}

#[test]
fn masks_lock_task_cli_entries_and_tolerances() {
    let options = super::EntryOptions {
        label: Some("Kunde".into()),
        tolerance: None,
        case_sensitive: Some(true),
        whole_word: Some(true),
    };
    let entry = store::new_entry("wörtlich", "Müller".into(), &options).unwrap();
    assert_eq!(entry.kind, crate::masks::EntryKind::Literal);
    assert!(entry.case_sensitive && entry.whole_word);
    assert_eq!(entry.label.as_deref(), Some("Kunde"));
    let fuzzy = store::new_entry(
        "fuzzy",
        "Schweinsbraten".into(),
        &super::EntryOptions::default(),
    )
    .unwrap();
    assert_eq!(fuzzy.tolerance, crate::masks::Tolerance::Medium);
    assert_eq!(
        store::tolerance("2/1,5").unwrap(),
        crate::masks::Tolerance::Custom {
            trim: 2,
            inner_tenths: 15
        }
    );
    assert!(store::tolerance("sehr").is_err());
    assert!(store::new_entry("glob", "x".into(), &options).is_err());
}

#[test]
fn masks_lock_task_cli_text_output_names_no_value() {
    let summary = crate::masks::ListSummary {
        id: crate::masks::ListId(3),
        name: "Kunden".into(),
        enabled: true,
        locked: true,
        scopes: vec!["/work/kunden".into()],
        entries: vec![crate::masks::EntrySummary {
            id: crate::masks::EntryId(9),
            kind: crate::masks::EntryKind::Fuzzy,
            label: "Name".into(),
            case_sensitive: false,
            whole_word: false,
            tolerance: crate::masks::Tolerance::High,
        }],
        assigned_values: 4,
    };
    let value = json!({"list": summary});
    let text = print::text(&value);
    assert!(
        text.starts_with("3\tKunden\taktiv\tgesperrt\t1 Einträge\t1 Schutzpfade\n"),
        "{text}"
    );
    assert!(text.contains("  schutzpfad\t/work/kunden\n"), "{text}");
    assert!(text.contains("  9\tfuzzy\tName\ttoleranz high\t"), "{text}");
    // The summary type has no value field at all.
    assert!(value["list"]["entries"][0].get("value").is_none());
    let status = print::text(
        &json!({"lockdown":"on","allowed_mcp_servers":[],"hardening":{"claude":{"state":"current","path":"/etc/x"}}}),
    );
    assert!(status.contains("agenten-sperre\ton\n"), "{status}");
    assert!(status.contains("erlaubte mcp-server\tkeine\n"), "{status}");
    assert!(status.contains("  claude\tcurrent\t/etc/x\n"), "{status}");
}
