//! Guarded paths: the app data and the agent host configuration are named
//! in every written form; ordinary commands are not.
use super::guarded::{guarded_overlap, guarded_paths, normalize, refers_to_guarded};

#[test]
fn masks_task_guarded_paths_cover_app_data_and_agent_hosts() {
    let paths = guarded_paths();
    let app = crate::support_dirs::app_data_dir();
    assert!(paths.contains(&app));
    let text: Vec<String> = paths
        .iter()
        .map(|p| normalize(&p.to_string_lossy()).to_lowercase())
        .collect();
    for host in ["claude", "codex"] {
        assert!(text.iter().any(|p| p.contains(host)), "{host}: {text:?}");
    }
}

#[test]
fn masks_task_commands_naming_guarded_paths_are_found() {
    let app = crate::support_dirs::app_data_dir();
    let store = app.join("masks").join("lists.sealed");
    let blocked = [
        format!("cat {}", store.display()),
        format!("cp \"{}\" /tmp/x", app.display()),
        "cat ~/.claude/settings.json".to_string(),
        "cd proj && sed -i s/x/y/ .claude/settings.local.json".to_string(),
        "echo '{}' > ~/.claude.json".to_string(),
        "type .codex\\config.toml".to_string(),
        "cat .co\"\"dex/hooks.json".to_string(),
        "ls ~/.local/share/smart_explorer/".to_string(),
    ];
    for command in &blocked {
        assert!(refers_to_guarded(command).is_some(), "{command}");
    }
    let allowed = [
        "cargo test -p smart_explorer_engine masks_task_",
        "cat src/main.rs",
        "ls ~/.claude-backup",
        "grep -r settings.json docs/",
        "git log --oneline",
    ];
    for command in allowed {
        assert_eq!(refers_to_guarded(command), None, "{command}");
    }
}

#[test]
fn masks_task_walks_over_parents_of_guarded_paths_overlap() {
    let app = crate::support_dirs::app_data_dir();
    let inside = app.join("masks");
    assert_eq!(guarded_overlap(&inside), Some(app.clone()));
    let parent = app.parent().unwrap().to_path_buf();
    assert!(guarded_overlap(&parent).is_some());
    let unrelated = tempfile::tempdir().unwrap();
    assert_eq!(guarded_overlap(unrelated.path()), None);
}
