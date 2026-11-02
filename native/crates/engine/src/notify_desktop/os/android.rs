//! Android delivers notices through daemon::set_problem_notifier.
pub(super) fn notify(_: &crate::syncjobs::ProblemNotice) -> Result<(), String> {
    Err("Android-Benachrichtigungshaken fehlt".into())
}
