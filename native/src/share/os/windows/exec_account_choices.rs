//! "Ausführen als" of a Windows worker (D.2.4, C2): the own account,
//! `se-remote` and every account the configuration lists for this worker's
//! user. Each listed account gets the result of the service's logon probe,
//! so the list shows what Windows really allows (Microsoft and Entra
//! accounts, Windows Home: A6, A7) and the measured rights (`elevated`,
//! `admin_group`, A9) instead of an assumption. The state of the service
//! comes first: not installed, installed (with its version), or in need of
//! an update.
use crate::share::access_catalog::{HelperState, HelperStatus};
use crate::share::access_grant::REMOTE_ACCOUNT_NAME;
use crate::share::exec_client_messages::ACCOUNT_NOT_ALLOWED;
use crate::share::exec_helper_protocol::{
    account_error_code, AccountChoice, AccountChoiceProfile, AccountChoices, AccountReason,
    HelperErrorCode,
};

use super::exec_account_client::{own_sid, OpenFailure, ProbeOutcome, ServiceConnection};
use super::exec_account_config::load_config;
use super::exec_account_scm::service_state;

const SETUP: &str = "as administrator: se share exec accounts setup --remote-account";
const UPDATE: &str = "as administrator: se share exec accounts update";

/// The choices of this worker with the state of the account service.
pub(crate) fn account_choices() -> AccountChoices {
    let own = std::env::var("USERNAME").unwrap_or_else(|_| "Windows user".into());
    let (helper, mut connection, allowed) = helper_state();
    let mut choices = vec![AccountChoice::own(&own)];
    let mut remote = AccountChoice::other(AccountChoiceProfile::Remote, REMOTE_ACCOUNT_NAME);
    assess(&mut remote, &helper, &allowed, connection.as_mut());
    choices.push(remote);
    for name in &allowed {
        if !name.eq_ignore_ascii_case(REMOTE_ACCOUNT_NAME) && !name.eq_ignore_ascii_case(&own) {
            let mut choice = AccountChoice::other(AccountChoiceProfile::Account, name);
            assess(&mut choice, &helper, &allowed, connection.as_mut());
            choices.push(choice);
        }
    }
    AccountChoices { helper, choices }
}

fn status(state: HelperState, version: Option<String>, detail: Option<String>) -> HelperStatus {
    HelperStatus {
        state,
        version,
        detail,
    }
}

/// The state of the service, a connection for the probes when it answers,
/// and the accounts the configuration lists for this worker's user.
fn helper_state() -> (HelperStatus, Option<ServiceConnection>, Vec<String>) {
    match service_state() {
        Ok(None) => {
            return (
                status(HelperState::NotInstalled, None, None),
                None,
                Vec::new(),
            )
        }
        Err(error) => {
            let detail = format!("the service control manager does not answer ({error})");
            return (
                status(HelperState::Outdated, None, Some(detail)),
                None,
                Vec::new(),
            );
        }
        Ok(Some(_)) => {}
    }
    let allowed = match (own_sid(), load_config()) {
        (Ok(sid), Ok(Some(config))) => config.accounts_for(&sid).to_vec(),
        (_, Err(error)) => {
            let detail = format!("its configuration is not usable ({error})");
            return (
                status(HelperState::Outdated, None, Some(detail)),
                None,
                Vec::new(),
            );
        }
        _ => Vec::new(),
    };
    let (helper, connection) = match ServiceConnection::open() {
        Ok(connection) => {
            let version = Some(connection.version().to_string());
            (
                status(HelperState::Installed, version, None),
                Some(connection),
            )
        }
        Err(OpenFailure::NotInstalled) => (status(HelperState::NotInstalled, None, None), None),
        Err(OpenFailure::NotRunning(error)) => {
            let detail = format!("it does not answer ({error})");
            (status(HelperState::Outdated, None, Some(detail)), None)
        }
        Err(OpenFailure::Refused(error)) => match account_error_code(&error) {
            Some(ACCOUNT_NOT_ALLOWED) => {
                let detail = format!("this user is not registered: {error}");
                (status(HelperState::Installed, None, Some(detail)), None)
            }
            _ => (
                status(HelperState::Outdated, None, Some(error.to_string())),
                None,
            ),
        },
    };
    (helper, connection, allowed)
}

/// Availability, reason and next step of one other account.
fn assess(
    choice: &mut AccountChoice,
    helper: &HelperStatus,
    allowed: &[String],
    connection: Option<&mut ServiceConnection>,
) {
    let name = choice.label.clone();
    let detail = helper.detail.clone().unwrap_or_default();
    let allow = format!("as administrator: se share exec accounts allow {name}");
    match helper.state {
        HelperState::NotInstalled => choice.refuse(
            AccountReason::HelperNotInstalled,
            "the account service is not set up on this PC".into(),
            Some(SETUP.into()),
        ),
        HelperState::Outdated => choice.refuse(
            AccountReason::HelperOutdated,
            format!("the account service needs an update ({detail})"),
            Some(UPDATE.into()),
        ),
        HelperState::NotNeeded | HelperState::Unavailable => {
            choice.refuse(AccountReason::Unsupported, detail, None)
        }
        HelperState::Installed if !allowed.iter().any(|item| item.eq_ignore_ascii_case(&name)) => {
            choice.refuse(
                AccountReason::NotAllowed,
                format!("account {name} is not allowed for the remote commands of this user"),
                Some(allow),
            )
        }
        HelperState::Installed => match connection.map(|connection| connection.probe(&name)) {
            None => choice.refuse(
                AccountReason::NotAllowed,
                format!("this user is not registered with the account service yet ({detail})"),
                Some(allow),
            ),
            Some(Ok(outcome)) => apply_probe(choice, outcome, allow),
            Some(Err(error)) => choice.refuse(
                AccountReason::HelperOutdated,
                format!("the account service did not answer the probe ({error})"),
                Some(UPDATE.into()),
            ),
        },
    }
}

fn apply_probe(choice: &mut AccountChoice, outcome: ProbeOutcome, allow: String) {
    let name = choice.label.clone();
    match outcome {
        ProbeOutcome::Measured {
            ok,
            elevated,
            admin_group,
            detail,
        } => {
            choice.elevated = Some(elevated);
            choice.admin_group = Some(admin_group);
            let detail = detail.unwrap_or_default();
            if ok {
                return;
            }
            let reason = if elevated || admin_group {
                AccountReason::Root
            } else {
                AccountReason::LogonRefused
            };
            choice.refuse(reason, detail, None);
        }
        ProbeOutcome::Refused(HelperErrorCode::UnknownAccount, detail) => {
            let missing_remote = name.eq_ignore_ascii_case(REMOTE_ACCOUNT_NAME);
            let next_step = missing_remote.then(|| SETUP.to_string());
            choice.refuse(AccountReason::NotFound, detail, next_step);
        }
        ProbeOutcome::Refused(HelperErrorCode::NotAllowed, detail) => {
            choice.refuse(AccountReason::NotAllowed, detail, Some(allow));
        }
        ProbeOutcome::Refused(HelperErrorCode::Outdated, detail) => {
            choice.refuse(AccountReason::HelperOutdated, detail, Some(UPDATE.into()));
        }
        ProbeOutcome::Refused(_, detail) => {
            choice.refuse(AccountReason::LogonRefused, detail, None);
        }
    }
}
