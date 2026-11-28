use super::PathFilter;
use crate::agent_proto::ContentQuery;

fn filter(filters: &str) -> PathFilter {
    let mut query = ContentQuery::text("x");
    query.filters = filters.to_string();
    PathFilter::new(&query).expect("valid filter")
}

/// Every example of the Notepad++ manual ("Find in Files", Filters), the empty
/// field and a field of exclusions only.
#[test]
fn content_search_task_filters_follow_notepad_plus_plus() {
    let sources = filter("*.cpp *.cxx *.h");
    assert!(sources.admit_file("main.cpp"));
    assert!(sources.admit_file("MAIN.CPP"));
    assert!(sources.admit_file("a.h"));
    assert!(!sources.admit_file("a.hpp"));
    assert!(!sources.admit_file("readme.txt"));
    assert!(sources.admit_folder("src", 1, "src", false));

    let no_build = filter("*.* !*.exe !*.obj !*.log");
    assert!(no_build.admit_file("notes.txt"));
    assert!(no_build.admit_file("Makefile"));
    assert!(!no_build.admit_file("app.EXE"));
    assert!(!no_build.admit_file("x.obj"));
    assert!(!no_build.admit_file("run.log"));

    let top = filter("*.* !\\tests !\\bin*");
    assert!(top.admit_file("a.rs"));
    assert!(!top.admit_folder("tests", 1, "tests", false));
    assert!(!top.admit_folder("binaries", 1, "binaries", false));
    assert!(!top.admit_folder("Bin", 1, "Bin", false));
    assert!(top.admit_folder("src/tests", 2, "tests", false));
    assert!(top.admit_folder("src", 1, "src", false));

    let every_level = filter("*.* !+\\log*");
    assert!(!every_level.admit_folder("logs", 1, "logs", false));
    assert!(!every_level.admit_folder("a/b/LogFiles", 3, "LogFiles", false));
    assert!(every_level.admit_folder("a/catalog", 2, "catalog", false));

    let only_exclusion = filter("!*.bin");
    assert!(only_exclusion.admit_file("a.txt"));
    assert!(only_exclusion.admit_file("noext"));
    assert!(!only_exclusion.admit_file("dump.bin"));

    let empty = filter("");
    assert!(empty.admit_file("anything"));
    assert!(empty.admit_file(".hidden"));
    let spaces = filter("   ");
    assert!(spaces.admit_file("x.y"));

    // DOS semantics of the wildcards.
    let stem = filter("make.*");
    assert!(stem.admit_file("make"));
    assert!(stem.admit_file("make.bat"));
    assert!(!stem.admit_file("maker"));
    let one = filter("?.txt");
    assert!(one.admit_file("a.txt"));
    assert!(!one.admit_file("ab.txt"));
    let middle = filter("*report*");
    assert!(middle.admit_file("Q3_Report_final"));
    // An exclusion wins over an inclusion of the same name.
    let both = filter("*.log !debug.log");
    assert!(both.admit_file("app.log"));
    assert!(!both.admit_file("DEBUG.LOG"));
}

/// The folder expressions: include picks the folders whose files are read
/// while the walk continues below the others, exclude does not enter.
#[test]
fn content_search_task_folder_regex_include_and_exclude() {
    let mut query = ContentQuery::text("x");
    query.folder_include = Some("^src(/|$)".into());
    query.folder_exclude = Some("(^|/)target$".into());
    let filter = PathFilter::new(&query).unwrap();
    assert!(!filter.search_in_folder(""));
    assert!(filter.search_in_folder("src"));
    assert!(filter.search_in_folder("SRC/core"));
    assert!(!filter.search_in_folder("docs"));
    assert!(filter.admit_folder("docs", 1, "docs", false));
    assert!(!filter.admit_folder("target", 1, "target", false));
    assert!(!filter.admit_folder("src/a/target", 3, "target", false));
    assert!(filter.admit_folder("src/targets", 2, "targets", false));

    // Hidden folders only with "In hidden folders"; hidden files always.
    assert!(!filter.admit_folder(".git", 1, ".git", true));
    query.hidden = true;
    assert!(PathFilter::new(&query)
        .unwrap()
        .admit_folder(".git", 1, ".git", true));
    assert!(filter.admit_file(".env"));

    // "In all sub-folders" off: only the files of the root.
    query.subfolders = false;
    assert!(!PathFilter::new(&query)
        .unwrap()
        .admit_folder("src", 1, "src", false));

    query.folder_include = Some("(".into());
    let error = PathFilter::new(&query).unwrap_err();
    assert!(error.contains("Ordner einschließen"), "{error}");
}
