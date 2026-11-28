//! What a launcher (pkexec, sudo) reported when it did not start the helper
//! (no ready frame): its exit status and error output decide the error.
//!
//! - pkexec (polkit 124, `man pkexec`): 126 the dialog was dismissed, 127 not
//!   authorized or another failure; without a registered agent it prints
//!   „No authentication agent found.“ (observed on Ubuntu 24.04 with
//!   `--disable-internal-agent`).
//! - `sudo -n` (`man sudo`): exit 1 with „a password is required“ when the
//!   account needs to authenticate, other texts for sudoers refusals.
use super::types::ElevateError;

/// Which launcher ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Launcher {
    Pkexec,
    SudoNonInteractive,
}

/// What the caller does next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The way does not work here; the next way (sudo -n after pkexec) may.
    TryNext(String),
    Stop(ElevateError),
}

/// Longest error text of a launcher kept in a message.
const MAX_TEXT: usize = 600;

pub fn classify(launcher: Launcher, code: Option<i32>, stderr: &str) -> Verdict {
    let text = short(stderr);
    match launcher {
        Launcher::Pkexec => match code {
            Some(126) => Verdict::Stop(ElevateError::Declined),
            Some(127) if stderr.contains("No authentication agent") => Verdict::TryNext(
                "no polkit authentication agent is registered for this session".into(),
            ),
            Some(127) if stderr.contains("Not authorized") => {
                Verdict::Stop(ElevateError::NotAllowed(text))
            }
            Some(127) => Verdict::Stop(ElevateError::NotAllowed(if text.is_empty() {
                "pkexec refused the start (exit 127)".into()
            } else {
                text
            })),
            other => Verdict::Stop(ElevateError::Failed(format!(
                "pkexec ended with {} before the helper started{}",
                status_text(other),
                suffix(&text)
            ))),
        },
        Launcher::SudoNonInteractive => {
            if stderr.contains("password is required") || stderr.contains("a terminal is required")
            {
                return Verdict::Stop(ElevateError::NoAgent(
                    "this session has no polkit agent and `sudo -n` needs a password for this account".into(),
                ));
            }
            if stderr.contains("not in the sudoers")
                || stderr.contains("is not allowed")
                || stderr.contains("may not run sudo")
            {
                return Verdict::Stop(ElevateError::NotAllowed(text));
            }
            Verdict::Stop(ElevateError::Failed(format!(
                "sudo -n ended with {} before the helper started{}",
                status_text(code),
                suffix(&text)
            )))
        }
    }
}

fn status_text(code: Option<i32>) -> String {
    code.map_or("a signal".to_string(), |code| format!("exit {code}"))
}

fn suffix(text: &str) -> String {
    if text.is_empty() {
        String::new()
    } else {
        format!(": {text}")
    }
}

fn short(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= MAX_TEXT {
        return text.to_string();
    }
    text.chars()
        .take(MAX_TEXT)
        .chain(std::iter::once('…'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_elevate_task_pkexec_and_sudo_exits_become_clear_errors() {
        assert_eq!(
            classify(Launcher::Pkexec, Some(126), ""),
            Verdict::Stop(ElevateError::Declined)
        );
        assert!(matches!(
            classify(
                Launcher::Pkexec,
                Some(127),
                "Error executing command as another user: No authentication agent found.\n"
            ),
            Verdict::TryNext(_)
        ));
        assert!(matches!(
            classify(
                Launcher::Pkexec,
                Some(127),
                "Error executing command as another user: Not authorized\n"
            ),
            Verdict::Stop(ElevateError::NotAllowed(text)) if text.contains("Not authorized")
        ));
        assert!(matches!(
            classify(Launcher::Pkexec, Some(1), "boom"),
            Verdict::Stop(ElevateError::Failed(text)) if text.contains("exit 1") && text.contains("boom")
        ));
        assert!(matches!(
            classify(
                Launcher::SudoNonInteractive,
                Some(1),
                "sudo: a password is required\n"
            ),
            Verdict::Stop(ElevateError::NoAgent(_))
        ));
        assert!(matches!(
            classify(
                Launcher::SudoNonInteractive,
                Some(1),
                "user is not in the sudoers file.  This incident will be reported.\n"
            ),
            Verdict::Stop(ElevateError::NotAllowed(_))
        ));
        let long = "x".repeat(2000);
        let Verdict::Stop(ElevateError::Failed(text)) =
            classify(Launcher::SudoNonInteractive, None, &long)
        else {
            panic!("a failure");
        };
        assert!(text.contains("a signal") && text.chars().count() < 700);
    }
}
