//! The origin of the Android entry points to a Share device (plan 2026-11-14,
//! E4, metadata presence). The entries open a device through the embedded
//! worker or the Android system; they cannot run in a host test, so these
//! checks read the entry's source text: what a person browses with names the
//! origin `User`, what runs in the background takes a handle of its own.

/// The text of the function that starts with `start`, up to the next
/// top-level function.
fn function<'a>(source: &'a str, start: &str) -> &'a str {
    let from = source
        .find(start)
        .unwrap_or_else(|| panic!("{start} is not in the source"));
    let rest = &source[from..];
    let after = &rest[start.len()..];
    let end = [
        "\nfn ",
        "\npub(super) fn ",
        "\npub(crate) fn ",
        "\n    pub(crate) fn ",
    ]
    .iter()
    .filter_map(|next| after.find(next))
    .min()
    .map_or(rest.len(), |at| at + start.len());
    &rest[..end]
}

#[test]
fn verbindungen_task_metadata_presence_the_file_views_of_the_app_browse_as_user() {
    let source = include_str!("pool.rs");
    let share = function(source, "fn open(plan: Plan)");
    let at = share.find("Plan::Share(target, path)").expect("share plan");
    let share = &share[at..share.find("Plan::Zip(").expect("zip plan")];
    assert!(share.contains("OperationOrigin::User"), "{share}");
    assert!(share.contains("open_share_in_process(&target, origin)"));
    assert!(share.contains("open_share_backend_for(target, origin)"));
    assert!(!share.contains("OperationOrigin::Job"));
    assert!(!share.contains("open_share_backend(target)"));
}

#[test]
fn verbindungen_task_metadata_presence_the_documents_provider_serves_a_person_as_user() {
    let device = function(include_str!("domains/docs_engines.rs"), "fn device_source(");
    assert!(device.contains("OperationOrigin::User"), "{device}");
    assert!(device.contains("open_share_backend_for("));
    assert!(!device.contains("open_share_backend("));
}

#[test]
fn verbindungen_task_metadata_presence_background_runs_take_a_job_handle() {
    let mirror = function(include_str!("domains/sync_run.rs"), "fn mirror_task(");
    assert_eq!(mirror.matches("rt.resolve_for_job(").count(), 2, "{mirror}");
    assert!(!mirror.contains("rt.resolve("));
    let walk = function(include_str!("domains/analyze.rs"), "fn resolve_remote(");
    assert!(walk.contains("crate::daemon::job_handle("), "{walk}");
    let runtime = function(include_str!("runtime.rs"), "pub(crate) fn resolve_for_job(");
    assert!(
        runtime.contains("crate::daemon::job_handle(&backend)"),
        "{runtime}"
    );
}

/// E4c: a view that reads itself again after an operation ended lists as
/// work of the app; the listing a person asks for does not.
#[test]
fn verbindungen_task_metadata_presence_a_reload_after_an_operation_lists_as_background() {
    let list = function(include_str!("fs_list.rs"), "pub(crate) fn list(");
    let scope = list
        .find("bool_or(args, \"background\", false).then(crate::vfs::BackgroundScope::enter)")
        .expect("the background argument opens the scope");
    let first_read = list.find("rt.resolve_loc(").expect("the pooled backend");
    assert!(
        scope < first_read,
        "the scope is held before the backend is asked"
    );
    assert!(
        list.contains("let _background ="),
        "bound, not dropped at once"
    );
}
