use super::{in_background, BackgroundScope};

#[test]
fn verbindungen_task_metadata_presence_background_scope_marks_only_its_thread() {
    assert!(!in_background());
    let outer = BackgroundScope::enter();
    assert!(in_background());
    let inner = BackgroundScope::enter();
    drop(inner);
    assert!(in_background(), "a nested scope ends before the outer one");
    let other = std::thread::spawn(in_background).join().unwrap();
    assert!(!other, "another thread is not in the scope");
    drop(outer);
    assert!(!in_background());
}
