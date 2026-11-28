//! `se languages list|add|remove|folder [--json]`: the language packs of the
//! text view (docs/oberflaechen/textansicht.md). Built-in languages come from
//! two-face (the syntax set of bat); added packs are Sublime syntax files
//! (`.sublime-syntax`, one file or a Sublime package folder) in the app data
//! folder `languages/`, where an added pack wins over a built-in language of
//! the same name or extension. The MCP tool `se` reaches the same command.
use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Args, Subcommand};
use serde_json::json;

use crate::textview::{LanguagePack, LanguageStore, PackOrigin, PackSource};

#[derive(Args, Debug)]
pub(super) struct LanguagesArgs {
    #[command(subcommand)]
    command: LanguagesCommand,
}

#[derive(Subcommand, Debug)]
enum LanguagesCommand {
    #[command(about = "Built-in and added languages with extensions, origin and license")]
    List {
        #[arg(long, help = "Only the added packs")]
        added: bool,
        #[arg(long)]
        json: bool,
    },
    #[command(
        about = "Add a language pack: a .sublime-syntax file or a folder of them (Sublime package)"
    )]
    Add {
        path: PathBuf,
        #[arg(
            long,
            help = "Pack name (default: file name without .sublime-syntax, or the folder name)"
        )]
        name: Option<String>,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Remove an added language pack")]
    Remove {
        name: String,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Print the language folder (and the cache folder)")]
    Folder {
        #[arg(long)]
        json: bool,
    },
}

/// Runs `se languages …` on the language folder in the app data folder.
pub(super) fn run(args: LanguagesArgs) -> Result<i32, String> {
    let store = LanguageStore::default_store();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    run_with(&store, args, &mut out)
}

/// [`run`] against an explicit store and writer.
pub(super) fn run_with(
    store: &LanguageStore,
    args: LanguagesArgs,
    out: &mut dyn Write,
) -> Result<i32, String> {
    let written = match args.command {
        LanguagesCommand::List { added, json } => {
            let packs: Vec<LanguagePack> = store
                .list()
                .into_iter()
                .filter(|p| !added || p.origin == PackOrigin::Added)
                .collect();
            if json {
                write_json(out, &json!({ "folder": store.folder(), "packs": packs }))
            } else {
                write_list(out, &packs)
            }
        }
        LanguagesCommand::Add { path, name, json } => {
            let pack = store
                .add(PackSource::Path(path), name.as_deref())
                .map_err(|e| e.to_string())?;
            if json {
                write_json(out, &json!({ "added": pack }))
            } else {
                writeln!(
                    out,
                    "Sprachpaket „{}“ hinzugefügt: {}",
                    pack.name,
                    languages_text(&pack)
                )
                .and_then(|()| writeln!(out, "Ordner: {}", store.folder().display()))
            }
        }
        LanguagesCommand::Remove { name, json } => {
            store.remove(&name).map_err(|e| e.to_string())?;
            if json {
                write_json(out, &json!({ "removed": name.trim() }))
            } else {
                writeln!(out, "Sprachpaket „{}“ entfernt", name.trim())
            }
        }
        LanguagesCommand::Folder { json } => {
            if json {
                write_json(
                    out,
                    &json!({ "folder": store.folder(), "cache": store.cache_folder() }),
                )
            } else {
                writeln!(out, "{}", store.folder().display())
            }
        }
    };
    match written.and_then(|()| out.flush()) {
        // A closed pipe (`se languages list | head`) is not an error.
        Err(error) if error.kind() != io::ErrorKind::BrokenPipe => {
            Err(format!("languages: Ausgabe fehlgeschlagen: {error}"))
        }
        _ => Ok(0),
    }
}

fn write_json(out: &mut dyn Write, value: &serde_json::Value) -> io::Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    writeln!(out, "{text}")
}

fn languages_text(pack: &LanguagePack) -> String {
    if pack.languages.is_empty() {
        return "keine Sprache".to_string();
    }
    pack.languages
        .iter()
        .map(|l| {
            if l.extensions.is_empty() {
                l.name.clone()
            } else {
                format!("{} ({})", l.name, l.extensions.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn write_list(out: &mut dyn Write, packs: &[LanguagePack]) -> io::Result<()> {
    for pack in packs {
        let origin = match pack.origin {
            PackOrigin::Builtin => "eingebaut",
            PackOrigin::Added => "hinzugefügt",
        };
        write!(
            out,
            "{origin}\t{}\t{}\t{}",
            pack.name,
            languages_text(pack),
            pack.license
        )?;
        if let Some(by) = &pack.shadowed_by {
            write!(out, "\tüberdeckt von „{by}“")?;
        }
        if let Some(path) = &pack.path {
            write!(out, "\t{}", path.display())?;
        }
        writeln!(out)?;
        for problem in &pack.problems {
            writeln!(out, "\tProblem: {problem}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::{Parser, Subcommand};

    use super::{run_with, LanguagesArgs, LanguagesCommand};
    use crate::textview::LanguageStore;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: TestCommand,
    }

    #[derive(Subcommand)]
    enum TestCommand {
        Languages(LanguagesArgs),
    }

    fn parse(argv: &[&str]) -> Result<LanguagesArgs, clap::Error> {
        let mut full = vec!["se", "languages"];
        full.extend_from_slice(argv);
        TestCli::try_parse_from(full).map(|cli| match cli.command {
            TestCommand::Languages(args) => args,
        })
    }

    #[test]
    fn languages_task_cli_arguments() {
        let args = parse(&["list", "--json", "--added"]).unwrap();
        assert!(matches!(
            args.command,
            LanguagesCommand::List {
                added: true,
                json: true
            }
        ));
        let args = parse(&["add", "pakete/Mein.sublime-syntax", "--name", "Mein"]).unwrap();
        match args.command {
            LanguagesCommand::Add { path, name, json } => {
                assert_eq!(path.to_string_lossy(), "pakete/Mein.sublime-syntax");
                assert_eq!(name.as_deref(), Some("Mein"));
                assert!(!json);
            }
            other => panic!("wrong command {other:?}"),
        }
        assert!(matches!(
            parse(&["remove", "Mein", "--json"]).unwrap().command,
            LanguagesCommand::Remove { json: true, .. }
        ));
        assert!(matches!(
            parse(&["folder"]).unwrap().command,
            LanguagesCommand::Folder { json: false }
        ));
        assert!(parse(&["add"]).is_err(), "add needs a path");
        assert!(parse(&["remove"]).is_err(), "remove needs a name");
        assert!(parse(&["unbekannt"]).is_err());
    }

    #[test]
    fn languages_task_cli_runs_against_store() {
        let root = tempfile::tempdir().unwrap();
        let store = LanguageStore::new(root.path().join("languages"), root.path().join("cache"));
        let mut out = Vec::new();
        run_with(&store, parse(&["folder", "--json"]).unwrap(), &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(
            value["folder"].as_str().unwrap(),
            store.folder().to_string_lossy()
        );

        let file = root.path().join("Mein.sublime-syntax");
        std::fs::write(
            &file,
            "%YAML 1.2\n---\nname: Mein\nfile_extensions: [mein]\nscope: source.mein\ncontexts:\n  main:\n    - match: 'x'\n      scope: keyword.mein\n",
        )
        .unwrap();
        let path = file.to_string_lossy().into_owned();
        let mut out = Vec::new();
        run_with(&store, parse(&["add", &path]).unwrap(), &mut out).unwrap();
        assert!(String::from_utf8_lossy(&out).contains("„Mein“ hinzugefügt"));

        let mut out = Vec::new();
        run_with(
            &store,
            parse(&["list", "--added", "--json"]).unwrap(),
            &mut out,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let packs = value["packs"].as_array().unwrap();
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0]["origin"], "added");
        assert_eq!(packs[0]["languages"][0]["extensions"][0], "mein");

        let mut out = Vec::new();
        let builtin = run_with(&store, parse(&["remove", "Rust"]).unwrap(), &mut out);
        assert!(builtin.unwrap_err().contains("eingebaute Sprache"));
        let mut out = Vec::new();
        run_with(&store, parse(&["remove", "Mein"]).unwrap(), &mut out).unwrap();
        assert!(String::from_utf8_lossy(&out).contains("entfernt"));
        let invalid = root.path().join("notiz.txt");
        std::fs::write(&invalid, "x").unwrap();
        let path = invalid.to_string_lossy().into_owned();
        let error = run_with(&store, parse(&["add", &path]).unwrap(), &mut Vec::new());
        assert!(error.unwrap_err().contains("keine .sublime-syntax-Datei"));
    }
}
