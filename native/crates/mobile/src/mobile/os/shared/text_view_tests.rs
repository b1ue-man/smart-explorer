//! `text.*` and `languages.*` (api.md §4.15, §4.16) through the dispatcher
//! of a runtime that is not installed globally: open, index, styled lines in
//! UTF-16 offsets, find, Markdown, close, the bounded document register and
//! the language packs in a temporary language folder.
use super::super::config::HostSettings;
use super::super::dispatch::dispatch;
use super::super::languages;
use super::*;
use std::path::PathBuf;
use std::time::Duration;

const MEIN: &str = "%YAML 1.2\n---\nname: MeinText\nfile_extensions: [meintxt]\nscope: source.meintext\ncontexts:\n  main:\n    - match: '\\bwort\\b'\n      scope: keyword.other.mein\n";

struct Fixture {
    dir: tempfile::TempDir,
    rt: Runtime,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("temp dir");
    let files = dir.path().join("files");
    let cache = dir.path().join("cache");
    for path in [&files, &cache] {
        std::fs::create_dir_all(path).expect("fixture dir");
    }
    let config = json!({ "filesDir": files, "cacheDir": cache, "startDaemon": false });
    let settings = HostSettings::parse(&config.to_string()).expect("settings");
    Fixture {
        dir,
        rt: Runtime::detached(settings),
    }
}

impl Fixture {
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.path().join(name);
        std::fs::write(&path, bytes).expect("write fixture file");
        path
    }

    fn call(&self, method: &str, args: Value) -> Result<Value, ApiError> {
        dispatch(&self.rt, method, &args)
    }

    fn open(&self, path: &std::path::Path) -> Value {
        self.call("text.open", json!({ "location": path.to_string_lossy() }))
            .expect("text.open")
    }

    /// Waits until the line index of `doc` is complete.
    fn indexed(&self, doc: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let status = self
                .call("text.status", json!({ "docId": doc }))
                .expect("status");
            if status["complete"] == true {
                return status;
            }
            assert!(Instant::now() < deadline, "index did not finish: {status}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn utf16(text: &str) -> u64 {
    text.encode_utf16().count() as u64
}

#[test]
fn textview_bridge_task_open_lines_status_close() {
    let fx = fixture();
    let text = "fn main() {\r\n    let größe = \"😀\";\r\n}\r\n";
    let path = fx.file("beispiel.rs", text.as_bytes());
    let opened = fx.open(&path);
    assert_eq!(opened["language"], "Rust");
    assert_eq!(opened["markdown"], false);
    assert_eq!(opened["encoding"], "UTF-8");
    assert_eq!(opened["name"], "beispiel.rs");
    assert_eq!(opened["prefix"], "");
    assert_eq!(opened["path"], path.to_string_lossy().as_ref());
    let doc = opened["docId"].as_str().expect("docId").to_string();

    let status = fx.indexed(&doc);
    assert_eq!(status["lines"], 4); // three line endings: four lines
    assert_eq!(status["lineEnding"], "CRLF");
    assert_eq!(status["mixedEndings"], false);
    assert_eq!(status["endings"]["crlf"], 3);

    let answer = fx
        .call(
            "text.lines",
            json!({ "docId": doc, "first": 1, "count": 5, "theme": "dark" }),
        )
        .expect("text.lines");
    assert_eq!(answer["language"], "Rust");
    assert_eq!(answer["first"], 1);
    assert_eq!(answer["complete"], true);
    let lines = answer["lines"].as_array().expect("lines");
    assert_eq!(lines.len(), 3, "lines 1..4 of 4");
    let second = &lines[0];
    assert_eq!(second["number"], 1);
    assert_eq!(second["text"], "    let größe = \"😀\";");
    // The spans cover the line without gaps, in UTF-16 offsets.
    let spans = second["spans"].as_array().expect("spans");
    assert_eq!(spans[0]["start"], 0);
    assert_eq!(
        spans.last().expect("span")["end"].as_u64(),
        Some(utf16("    let größe = \"😀\";"))
    );
    for pair in spans.windows(2) {
        assert_eq!(pair[0]["end"], pair[1]["start"]);
    }
    assert!(
        spans.iter().any(|span| span["fg"] != spans[0]["fg"]),
        "{spans:?}"
    );
    assert!(answer["colors"]["background"].as_u64().is_some());

    // Another language for the same document: plain text is one span.
    let plain = fx
        .call(
            "text.lines",
            json!({ "docId": doc, "first": 0, "count": 1, "theme": "light", "language": "Plain Text" }),
        )
        .expect("plain lines");
    assert_eq!(plain["language"], PLAIN_TEXT);
    assert_eq!(plain["lines"][0]["spans"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        fx.call("text.lines", json!({ "docId": doc, "theme": "lila" }))
            .map_err(|e| e.kind),
        Err("invalid")
    );

    assert_eq!(
        fx.call("text.close", json!({ "docId": doc }))
            .expect("close")["closed"],
        true
    );
    assert_eq!(
        fx.call("text.lines", json!({ "docId": doc }))
            .map_err(|e| e.kind),
        Err("not_found")
    );
    assert_eq!(
        fx.call("text.close", json!({ "docId": doc }))
            .expect("close")["closed"],
        false
    );
}

#[test]
fn textview_bridge_task_binary_and_folder_are_refused() {
    let fx = fixture();
    let binary = fx.file("bild.dat", b"PNG\0\0\x01\x02 hallo");
    let refused = fx
        .call("text.open", json!({ "location": binary.to_string_lossy() }))
        .expect_err("binary file");
    assert_eq!(refused.kind, "unsupported");
    assert!(
        refused.message.contains("keine Textdatei"),
        "{}",
        refused.message
    );
    let folder = fx
        .call(
            "text.open",
            json!({ "location": fx.dir.path().to_string_lossy() }),
        )
        .expect_err("folder");
    assert_eq!(folder.kind, "invalid");
    let missing = fx.dir.path().join("fehlt.txt");
    assert_eq!(
        fx.call(
            "text.open",
            json!({ "location": missing.to_string_lossy() })
        )
        .map_err(|e| e.kind),
        Err("not_found")
    );
}

#[test]
fn textview_bridge_task_find_forward_backward_wrap() {
    let fx = fixture();
    let text = "Äpfel und Birnen\nKEIN treffer\nmehr äpfel 😀 äpfel\nende";
    let doc = fx.open(&fx.file("obst.txt", text.as_bytes()))["docId"]
        .as_str()
        .expect("docId")
        .to_string();
    let find = |args: Value| -> Value {
        let mut args = args;
        args["docId"] = json!(doc);
        fx.call("text.find", args).expect("text.find")
    };
    // Case-insensitive by default; the Ä is one UTF-16 unit.
    let first = find(json!({ "pattern": "äpfel" }));
    assert_eq!(
        (
            first["line"].as_u64(),
            first["start"].as_u64(),
            first["end"].as_u64()
        ),
        (Some(0), Some(0), Some(5))
    );
    assert_eq!(first["wrapped"], false);
    let next = find(json!({ "pattern": "äpfel", "fromLine": 0, "fromColumn": 5 }));
    assert_eq!(
        (next["line"].as_u64(), next["start"].as_u64()),
        (Some(2), Some(5))
    );
    // After the emoji (two UTF-16 units): "mehr äpfel 😀 " is 14 units.
    let third = find(json!({ "pattern": "äpfel", "fromLine": 2, "fromColumn": 10 }));
    assert_eq!(
        (
            third["line"].as_u64(),
            third["start"].as_u64(),
            third["end"].as_u64()
        ),
        (Some(2), Some(14), Some(19))
    );
    // At the end it continues at the top.
    let wrapped = find(json!({ "pattern": "äpfel", "fromLine": 2, "fromColumn": 19 }));
    assert_eq!(
        (wrapped["line"].as_u64(), wrapped["wrapped"].as_bool()),
        (Some(0), Some(true))
    );
    assert_eq!(
        find(json!({ "pattern": "äpfel", "fromLine": 2, "fromColumn": 19, "wrap": false }))
            ["found"],
        false
    );
    // Backwards from the third: the second, then (wrapping) the last.
    let back =
        find(json!({ "pattern": "äpfel", "fromLine": 2, "fromColumn": 14, "backward": true }));
    assert_eq!(
        (back["line"].as_u64(), back["start"].as_u64()),
        (Some(2), Some(5))
    );
    let around =
        find(json!({ "pattern": "äpfel", "fromLine": 0, "fromColumn": 0, "backward": true }));
    assert_eq!(
        (
            around["line"].as_u64(),
            around["start"].as_u64(),
            around["wrapped"].as_bool()
        ),
        (Some(2), Some(14), Some(true))
    );
    // Match case, whole word and regular expressions as in the content search.
    assert_eq!(
        find(json!({ "pattern": "kein", "matchCase": true }))["found"],
        false
    );
    assert_eq!(
        find(json!({ "pattern": "pfel", "wholeWord": true }))["found"],
        false
    );
    let regex = find(json!({ "pattern": "^ende$", "mode": "regex" }));
    assert_eq!(
        (regex["line"].as_u64(), regex["start"].as_u64()),
        (Some(3), Some(0))
    );
    let invalid = fx
        .call(
            "text.find",
            json!({ "docId": doc, "pattern": "(", "mode": "regex" }),
        )
        .expect_err("invalid regex");
    assert_eq!(invalid.kind, "invalid");
}

#[test]
fn textview_bridge_task_markdown_document_and_text() {
    let fx = fixture();
    let text = "# Titel\n\nText mit **fett** und [Link](andere.md).\n\n```rust\nlet ü = 1;\n```\n\n- [x] erledigt\n";
    let opened = fx.open(&fx.file("LIESMICH.md", text.as_bytes()));
    assert_eq!(opened["markdown"], true);
    let doc = opened["docId"].as_str().expect("docId");
    let rendered = fx
        .call("text.markdown", json!({ "docId": doc, "theme": "light" }))
        .expect("markdown");
    let blocks = rendered["blocks"].as_array().expect("blocks");
    assert_eq!(blocks[0]["type"], "heading");
    assert_eq!(blocks[0]["level"], 1);
    let paragraph = blocks[1]["runs"].as_array().expect("runs");
    assert!(paragraph
        .iter()
        .any(|run| run["strong"] == true && run["text"] == "fett"));
    assert!(paragraph.iter().any(|run| run["link"] == "andere.md"));
    let code = blocks
        .iter()
        .find(|b| b["type"] == "code")
        .expect("code block");
    assert_eq!(code["highlightedAs"], "Rust");
    let line = &code["lines"][0];
    assert_eq!(line["text"], "let ü = 1;");
    assert_eq!(
        line["spans"]
            .as_array()
            .and_then(|s| s.last())
            .map(|s| s["end"].clone()),
        Some(json!(10))
    );
    let list = blocks.iter().find(|b| b["type"] == "list").expect("list");
    assert_eq!(list["items"][0]["task"], true);
    // Text without a document (a preview).
    let direct = fx
        .call(
            "text.markdown",
            json!({ "text": "> Zitat", "theme": "dark" }),
        )
        .expect("markdown text");
    assert_eq!(direct["blocks"][0]["type"], "quote");
}

#[test]
fn textview_bridge_task_register_closes_least_recently_used() {
    let fx = fixture();
    let path = fx.file("a.txt", b"eins\nzwei\n");
    let ids: Vec<String> = (0..MAX_DOCS)
        .map(|_| fx.open(&path)["docId"].as_str().expect("docId").to_string())
        .collect();
    std::thread::sleep(Duration::from_millis(5));
    // The first is used again; the second is now the least recently used.
    fx.call("text.status", json!({ "docId": ids[0] }))
        .expect("status");
    let newest = fx.open(&path)["docId"].as_str().expect("docId").to_string();
    let open = fx.rt.inner.texts.ids();
    assert_eq!(open.len(), MAX_DOCS);
    assert!(open.contains(&ids[0]));
    assert!(!open.contains(&ids[1]), "{open:?}");
    assert!(open.contains(&newest));
    assert_eq!(
        fx.call("text.status", json!({ "docId": ids[1] }))
            .map_err(|e| e.kind),
        Err("not_found")
    );
}

#[test]
fn textview_bridge_task_language_packs_add_list_remove() {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let root = tempfile::tempdir().expect("temp dir");
    let store = crate::textview::LanguageStore::new(
        root.path().join("languages"),
        root.path().join("cache"),
    );
    let add = |args: Value| languages::add(&store, &args);
    let added =
        add(json!({ "fileName": "mein.sublime-syntax", "bytesBase64": STANDARD.encode(MEIN) }))
            .expect("add");
    assert_eq!(added["name"], "mein");
    assert_eq!(added["origin"], "added");
    assert_eq!(added["languages"][0]["name"], "MeinText");
    assert_eq!(added["languages"][0]["extensions"][0], "meintxt");
    assert!(added["path"]
        .as_str()
        .is_some_and(|path| path.ends_with("mein.sublime-syntax")));
    let list = languages::list(&store);
    let entries = list.as_array().expect("list");
    assert_eq!(entries[0]["name"], "mein");
    let rust = entries
        .iter()
        .find(|p| p["name"] == "Rust")
        .expect("Rust built in");
    assert_eq!(rust["origin"], "builtin");
    assert!(rust["path"].is_null());

    let again =
        add(json!({ "fileName": "mein.sublime-syntax", "bytesBase64": STANDARD.encode(MEIN) }));
    assert_eq!(again.map_err(|e| e.kind), Err("exists"));
    let broken = add(
        json!({ "fileName": "kaputt.sublime-syntax", "bytesBase64": STANDARD.encode("name: [") }),
    );
    assert_eq!(broken.map_err(|e| e.kind), Err("invalid"));
    let garbage = add(json!({ "fileName": "x.sublime-syntax", "bytesBase64": "%%%" }));
    assert_eq!(garbage.map_err(|e| e.kind), Err("invalid"));
    let builtin = languages::remove(&store, &json!({ "name": "Rust" }));
    assert_eq!(builtin.map_err(|e| e.kind), Err("invalid"));
    languages::remove(&store, &json!({ "name": "mein" })).expect("remove");
    assert!(!root.path().join("languages/mein.sublime-syntax").exists());
    let gone = languages::remove(&store, &json!({ "name": "mein" }));
    assert_eq!(gone.map_err(|e| e.kind), Err("not_found"));

    // The dispatcher answers the lists of the app's own folder.
    let fx = fixture();
    let packs = fx
        .call("languages.list", json!({}))
        .expect("languages.list");
    assert!(packs
        .as_array()
        .is_some_and(|p| p.iter().any(|p| p["name"] == "Rust")));
    let names = fx
        .call("text.languages", json!({}))
        .expect("text.languages");
    assert!(names.as_array().is_some_and(|l| l
        .iter()
        .any(|l| l["name"] == "Rust" && l["extensions"].as_array().is_some())));
}
