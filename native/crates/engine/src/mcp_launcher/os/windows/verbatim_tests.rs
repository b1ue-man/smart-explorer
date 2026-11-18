//! The root check accepts the plain and the canonical (`\\?\`) form of the
//! same installation on either side (ASUS 2026-11-18: a rollback found by the
//! app stayed `runtime_path_outside_installation` forever).
use super::*;

#[test]
fn update_root_task_plain_and_verbatim_roots_are_the_same_installation() {
    let plain = Path::new(r"C:\Users\Someone\AppData\Local\Programs\Smart Explorer");
    let canonical = Path::new(r"\\?\C:\Users\Someone\AppData\Local\Programs\Smart Explorer");
    assert_eq!(verbatim(plain), canonical);
    assert_eq!(verbatim(canonical), canonical);
    assert!(verbatim(&canonical.join("se.exe")).starts_with(verbatim(plain)));
    assert!(verbatim(&plain.join("se.exe")).starts_with(verbatim(canonical)));
    let outside = Path::new(r"C:\Users\Someone\AppData\Local\Programs\Other\se.exe");
    assert_eq!(
        validate_tree(canonical, outside).unwrap_err(),
        "runtime_path_outside_installation"
    );
    let unc = Path::new(r"\\server\share\Smart Explorer");
    assert_eq!(verbatim(unc), unc);
}
