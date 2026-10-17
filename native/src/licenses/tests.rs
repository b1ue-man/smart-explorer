//! INT-LIC acceptance signal of the embedding (plan C4). Suite job
//! `linux-core`, stage `unit`, after the job `prepare` placed the generated
//! file in the checkout (L2).

use std::collections::BTreeSet;
use std::path::PathBuf;

fn license_file() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../third-party/rust-crates/THIRD-PARTY-LICENSES.txt")
}

/// Crate lines read without the module parser: the first two words of every
/// line of the crate list.
fn crate_names_on_disk(text: &str) -> BTreeSet<(String, String)> {
    let text = text.replace("\r\n", "\n");
    let start = text
        .find(super::CRATES_HEADING)
        .expect("crate list heading in the license file");
    let section = &text[start + super::CRATES_HEADING.len()..];
    let end = section
        .find(&format!("\n{}\n", super::TEXTS_HEADING))
        .expect("license text heading after the crate list");
    section[..end]
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| !line.starts_with('[') && !line.starts_with('-'))
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            Some((words.next()?.to_string(), words.next()?.to_string()))
        })
        .collect()
}

#[test]
fn licenses_task_embedded_text_matches_file() {
    let path = license_file();
    let on_disk = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{}: {error}; the suite job prepare places the generated file before the build (L2)",
            path.display()
        )
    });
    let embedded = super::text().unwrap_or_else(|| {
        panic!(
            "{} exists, but the build embedded nothing: native/build.rs did not set the cfg \
             se_crate_licenses (was the file placed after the build?)",
            path.display()
        )
    });
    assert!(
        embedded == on_disk,
        "embedded text ({} bytes) differs from {} ({} bytes)",
        embedded.len(),
        path.display(),
        on_disk.len()
    );
    assert_eq!(
        super::sha256().as_deref(),
        Some(super::lf_sha256(&on_disk).as_str()),
        "sha256() must hash the file with LF line ends"
    );

    let parsed = super::crates();
    assert!(
        !parsed.is_empty(),
        "no crate line parsed from the embedded file"
    );
    for entry in &parsed {
        assert!(
            !entry.name.is_empty() && !entry.version.is_empty() && !entry.license.trim().is_empty(),
            "incomplete crate line: {entry:?}"
        );
    }
    let parsed_names: BTreeSet<(String, String)> = parsed
        .iter()
        .map(|entry| (entry.name.clone(), entry.version.clone()))
        .collect();
    assert_eq!(
        parsed_names.len(),
        parsed.len(),
        "crates() lists a crate twice"
    );
    assert_eq!(
        parsed_names,
        crate_names_on_disk(&on_disk),
        "crates() and the crate list of the file differ"
    );
}
