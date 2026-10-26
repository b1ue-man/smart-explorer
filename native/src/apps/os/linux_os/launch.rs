use super::super::shared::platform::{expired, DispatchReceipt, HostPlatform, PlatformCatalog};
use super::{
    catalog::{self, Command as Start, Target},
    session,
};
use crate::apps::core::model::{HostSession, LaunchRequest, LaunchState};
use std::{collections::HashMap, io, process::Command, time::Duration};

#[derive(Default)]
pub(crate) struct Platform;
fn outcome(state: LaunchState, reason: impl Into<String>, pid: Option<u32>) -> DispatchReceipt {
    DispatchReceipt {
        state,
        reason: reason.into(),
        pid,
    }
}
impl HostPlatform for Platform {
    type Target = Target;
    fn session(&self) -> io::Result<HostSession> {
        session::current()
    }
    fn snapshot(&self) -> io::Result<PlatformCatalog<Target>> {
        let session = session::current()?;
        let apps = catalog::list()?;
        // Enumeration may block on the filesystem. A logout during it invalidates
        // the whole result instead of publishing a mixed-login catalog.
        if session::current()? != session {
            return Err(io::Error::other(
                "graphical session changed while listing applications",
            ));
        }
        Ok(PlatformCatalog {session,apps,limits:vec!["GUI launch requires this user's active logind graphical session; root and detached service workers are unsupported".into()]})
    }
    fn dispatch(
        &self,
        target: &Target,
        request: &LaunchRequest,
        expected: &HostSession,
        deadline: Option<i64>,
        dispatch_id: &str,
    ) -> io::Result<DispatchReceipt> {
        if !session::current().is_ok_and(|current| &current == expected) {
            return Ok(outcome(
                LaunchState::Denied,
                "host_login_session_changed_or_unavailable",
                None,
            ));
        }
        if !expected.unlocked {
            return Ok(outcome(
                LaunchState::ConsentRequired,
                "unlock_the_host_graphical_session",
                None,
            ));
        }
        if !catalog::read(&target.file).is_ok_and(|text| catalog::hash(&text) == target.source_hash)
        {
            return Ok(outcome(
                LaunchState::Denied,
                "installed_desktop_entry_changed",
                None,
            ));
        }
        match &target.command {
            Start::Unavailable => Ok(outcome(
                LaunchState::Unsupported,
                "desktop_entry_is_not_launchable",
                None,
            )),
            Start::Exec {
                program,
                args,
                directory,
                executable_stamp,
            } => {
                if !catalog::stamp(program).is_ok_and(|stamp| &stamp == executable_stamp) {
                    return Ok(outcome(
                        LaunchState::Denied,
                        "installed_executable_changed_or_unavailable",
                        None,
                    ));
                }
                let mut command = Command::new(program);
                command.args(args);
                if let Some(directory) = directory {
                    command.current_dir(directory);
                }
                command
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                let witness = super::window_correlation::prepare(&mut command).ok();
                if !session::current().is_ok_and(|current| &current == expected) {
                    return Ok(outcome(
                        LaunchState::Denied,
                        "host_login_session_changed_or_unavailable",
                        None,
                    ));
                }
                if expired(deadline) {
                    return Ok(outcome(
                        LaunchState::Denied,
                        "app_launch_grant_expired",
                        None,
                    ));
                }
                match command.spawn() {
                    Ok(mut child) => {
                        let pid = child.id();
                        super::window_correlation::record(
                            witness.as_ref(),
                            &mut child,
                            request,
                            expected,
                            dispatch_id,
                        );
                        let _reaper = std::thread::Builder::new()
                            .name("app-launch-reaper".into())
                            .spawn(move || {
                                let _ = child.wait();
                            });
                        Ok(outcome(
                            LaunchState::StartedWindowPending,
                            "started_window_not_yet_available",
                            Some(pid),
                        ))
                    }
                    Err(e) => Ok(outcome(
                        LaunchState::Denied,
                        format!("exec_spawn_refused: {e}"),
                        None,
                    )),
                }
            }
            Start::Dbus { service, path } => {
                let connection = zbus::blocking::connection::Builder::session()
                    .map_err(io::Error::other)?
                    .method_timeout(Duration::from_secs(5))
                    .build()
                    .map_err(io::Error::other)?;
                let proxy = zbus::blocking::Proxy::new(
                    &connection,
                    service.as_str(),
                    path.as_str(),
                    "org.freedesktop.Application",
                )
                .map_err(io::Error::other)?;
                let options: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
                if !session::current().is_ok_and(|current| &current == expected) {
                    return Ok(outcome(
                        LaunchState::Denied,
                        "host_login_session_changed_or_unavailable",
                        None,
                    ));
                }
                if expired(deadline) {
                    return Ok(outcome(
                        LaunchState::Denied,
                        "app_launch_grant_expired",
                        None,
                    ));
                }
                let _: () = proxy
                    .call("Activate", &(options,))
                    .map_err(io::Error::other)?;
                // D-Bus acknowledgment proves activation was accepted, not that
                // a window was focused or even created. P4 supplies that proof.
                Ok(outcome(
                    LaunchState::StartedWindowPending,
                    "application_activation_accepted_window_pending",
                    None,
                ))
            }
        }
    }
}
