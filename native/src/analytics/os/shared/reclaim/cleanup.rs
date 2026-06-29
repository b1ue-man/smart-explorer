use std::path::Path;

use super::types::ReclaimConfidence;

#[derive(Clone, Copy)]
pub(crate) struct CleanupDecision {
    pub reason: &'static str,
    pub confidence: ReclaimConfidence,
}

pub(crate) fn file_cleanup_reason(name: &str) -> Option<CleanupDecision> {
    let n = name.to_ascii_lowercase();
    if n.ends_with(".log") {
        Some(CleanupDecision {
            reason: "Logdatei",
            confidence: ReclaimConfidence::ReviewSafe,
        })
    } else {
        None
    }
}

pub(crate) fn dir_cleanup_reason(dir: &Path, root: &Path) -> Option<CleanupDecision> {
    let name = dir.file_name()?.to_string_lossy();
    dir_cleanup_by_name(&name, dir.parent(), dir != root)
}

pub(crate) fn remote_dir_cleanup_reason(name: &str) -> Option<CleanupDecision> {
    dir_cleanup_by_name(name, None, true)
}

fn dir_cleanup_by_name(
    name: &str,
    parent: Option<&Path>,
    allow_root_cleanup: bool,
) -> Option<CleanupDecision> {
    if !allow_root_cleanup {
        return None;
    }
    let n = name.to_ascii_lowercase();
    match n.as_str() {
        ".git" => Some(CleanupDecision {
            reason: ".git",
            confidence: ReclaimConfidence::NeverAuto,
        }),
        "node_modules" => Some(CleanupDecision {
            reason: "node_modules",
            confidence: if parent.is_some_and(has_node_project_context) {
                ReclaimConfidence::ReviewSafe
            } else {
                ReclaimConfidence::RiskyReview
            },
        }),
        "target" => Some(CleanupDecision {
            reason: "Rust-Build",
            confidence: if parent.is_some_and(|p| p.join("Cargo.toml").is_file()) {
                ReclaimConfidence::ReviewSafe
            } else {
                ReclaimConfidence::RiskyReview
            },
        }),
        "build" | "dist" => Some(CleanupDecision {
            reason: "Build-Ausgabe",
            confidence: if parent.is_some_and(has_build_context) {
                ReclaimConfidence::ReviewSafe
            } else {
                ReclaimConfidence::RiskyReview
            },
        }),
        "cache" | "caches" | ".cache" => Some(CleanupDecision {
            reason: "Cache",
            confidence: ReclaimConfidence::RiskyReview,
        }),
        "log" | "logs" => Some(CleanupDecision {
            reason: "Logs",
            confidence: ReclaimConfidence::ReviewSafe,
        }),
        "__pycache__" | ".pytest_cache" | ".mypy_cache" => Some(CleanupDecision {
            reason: "Python-Cache",
            confidence: ReclaimConfidence::ReviewSafe,
        }),
        ".gradle" => Some(CleanupDecision {
            reason: "Gradle-Cache",
            confidence: ReclaimConfidence::RiskyReview,
        }),
        _ => None,
    }
}

fn has_node_project_context(parent: &Path) -> bool {
    parent.join("package.json").is_file()
        && [
            "package-lock.json",
            "npm-shrinkwrap.json",
            "yarn.lock",
            "pnpm-lock.yaml",
        ]
        .iter()
        .any(|name| parent.join(name).is_file())
}

fn has_build_context(parent: &Path) -> bool {
    parent.join("Cargo.toml").is_file()
        || parent.join("package.json").is_file()
        || parent.join("pyproject.toml").is_file()
        || parent.join("build.gradle").is_file()
        || parent.join("build.gradle.kts").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_is_never_auto() {
        let base = std::env::temp_dir().join(format!("se_cleanup_git_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join(".git")).unwrap();
        let d = dir_cleanup_reason(&base.join(".git"), &base).unwrap();
        assert_eq!(d.confidence, ReclaimConfidence::NeverAuto);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn node_modules_needs_lockfile_for_quick_select() {
        let base = std::env::temp_dir().join(format!("se_cleanup_node_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("node_modules")).unwrap();
        assert_eq!(
            dir_cleanup_reason(&base.join("node_modules"), &base)
                .unwrap()
                .confidence,
            ReclaimConfidence::RiskyReview
        );
        std::fs::write(base.join("package.json"), "{}").unwrap();
        std::fs::write(base.join("package-lock.json"), "{}").unwrap();
        assert_eq!(
            dir_cleanup_reason(&base.join("node_modules"), &base)
                .unwrap()
                .confidence,
            ReclaimConfidence::ReviewSafe
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cargo_target_is_quick_select_only_in_cargo_project() {
        let base = std::env::temp_dir().join(format!("se_cleanup_target_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("target")).unwrap();
        assert_eq!(
            dir_cleanup_reason(&base.join("target"), &base)
                .unwrap()
                .confidence,
            ReclaimConfidence::RiskyReview
        );
        std::fs::write(base.join("Cargo.toml"), "[package]\nname='x'\n").unwrap();
        assert_eq!(
            dir_cleanup_reason(&base.join("target"), &base)
                .unwrap()
                .confidence,
            ReclaimConfidence::ReviewSafe
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
