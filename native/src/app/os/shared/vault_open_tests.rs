use super::*;
use std::cell::RefCell;

#[test]
fn vault_task_open_fallback_requires_consent() {
    let location = "vault://0123456789abcdef0123456789abcdef/Plan.txt";
    let opened = RefCell::new(Vec::new());
    let asked = RefCell::new(Vec::new());
    let open_local = |path: &PathBuf| opened.borrow_mut().push(path.clone());
    let ask = |answer: bool| {
        let asked = &asked;
        move |title: &str, text: &str| {
            asked.borrow_mut().push(format!("{title}|{text}"));
            answer
        }
    };
    let mounted = |_: &str| -> Result<Option<PathBuf>, String> {
        Ok(Some(PathBuf::from(
            "/run/user/1000/smart-explorer/Tresore/A/Plan.txt",
        )))
    };
    let unmounted = |_: &str| -> Result<Option<PathBuf>, String> { Ok(None) };
    let mounts = |_: &str| -> Result<PathBuf, String> { Ok(PathBuf::from("T:\\Plan.txt")) };
    let no_runtime =
        |_: &str| -> Result<PathBuf, String> { Err(format!("{}: keine Laufwerkslaufzeit", crate::vault::MOUNT_RUNTIME_MISSING)) };
    let result = open_vault_file_with(
        location,
        true,
        &mounted,
        &no_runtime,
        &open_local,
        &ask(true),
    );
    assert_eq!(result, VaultFileOpen::Opened);
    let result =
        open_vault_file_with(location, true, &unmounted, &mounts, &open_local, &ask(true));
    assert_eq!(result, VaultFileOpen::Opened);
    assert_eq!(opened.borrow().len(), 2);
    assert!(
        asked.borrow().is_empty(),
        "no copy question while a drive opens"
    );
    let result = open_vault_file_with(
        location,
        true,
        &unmounted,
        &no_runtime,
        &open_local,
        &ask(false),
    );
    assert_eq!(result, VaultFileOpen::Declined);
    let result = open_vault_file_with(
        location,
        true,
        &unmounted,
        &no_runtime,
        &open_local,
        &ask(true),
    );
    assert_eq!(result, VaultFileOpen::CopyAgreed);
    let result = open_vault_file_with(
        location,
        false,
        &unmounted,
        &mounts,
        &open_local,
        &ask(false),
    );
    assert_eq!(result, VaultFileOpen::Declined);
    assert_eq!(opened.borrow().len(), 2, "nothing opened without a drive");
    let questions = asked.borrow();
    assert_eq!(questions.len(), 2);
    assert!(questions
        .iter()
        .all(|question| question.starts_with(COPY_TITLE)));
    assert!(questions[0].contains("keine Laufwerkslaufzeit"));
    let broken = |_: &str| -> Result<Option<PathBuf>, String> { Err("IPC".into()) };
    assert_eq!(
        open_step(location, true, &broken, &mounts),
        VaultOpenStep::Blocked("IPC".into())
    );
}
