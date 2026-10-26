//! Context-owned account loading and explicit administrator actions.
use super::{page, AccountsPage, PageAction};
use crate::share::{ExecAccountsAction, ExecAccountsElevation};
use eframe::egui;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

type Shared = Arc<Mutex<AccountsPage>>;

fn shared(ctx: &egui::Context) -> Shared {
    let id = egui::Id::new("share_access_accounts");
    ctx.data_mut(|data| {
        data.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(AccountsPage::new())))
            .clone()
    })
}

fn lock(state: &Shared) -> MutexGuard<'_, AccountsPage> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The section of the tab „Befehle“: reads the list, draws the page and
/// runs what was clicked.
pub(in crate::app) fn ui_accounts(ui: &mut egui::Ui) {
    let state = shared(ui.ctx());
    let unread = {
        let page = lock(&state);
        page.choices.is_none() && !page.reading
    };
    if unread {
        read(ui.ctx(), &state);
    }
    let session_limits = crate::share::exec_account_session_limits();
    let action = page(ui, &mut lock(&state), session_limits);
    match action {
        Some(PageAction::Refresh) => read(ui.ctx(), &state),
        Some(PageAction::CopyCommand(command)) => ui.ctx().copy_text(command),
        Some(PageAction::Apply(apply)) => run(ui.ctx(), &state, apply),
        None => {}
    }
}

/// Reads the list in the background: the worker's, else this process's.
fn read(ctx: &egui::Context, state: &Shared) {
    lock(state).reading = true;
    let (thread_state, thread_ctx) = (state.clone(), ctx.clone());
    let spawned = std::thread::Builder::new()
        .name("share-access-accounts".into())
        .spawn(move || {
            let choices = match crate::daemon::access_accounts() {
                Ok((helper, choices)) => AccountChoices { helper, choices },
                Err(_) => crate::share::exec_account_choices(),
            };
            let mut page = lock(&thread_state);
            page.choices = Some(choices);
            page.reading = false;
            thread_ctx.request_repaint();
        });
    if spawned.is_err() {
        lock(state).reading = false;
    }
}

/// Runs one administrator action in the background, then reads the list
/// again.
fn run(ctx: &egui::Context, state: &Shared, action: ExecAccountsAction) {
    let name = action_label(&action);
    {
        let mut page = lock(state);
        if page.busy.is_some() {
            return;
        }
        page.busy = Some(name);
        page.error = None;
        page.outcome = None;
    }
    let (thread_state, thread_ctx) = (state.clone(), ctx.clone());
    let spawned = std::thread::Builder::new()
        .name("share-access-accounts-setup".into())
        .spawn(move || {
            let elevation = ExecAccountsElevation::PkexecGraphical;
            let result = crate::share::exec_accounts_apply(&action, elevation);
            {
                let mut page = lock(&thread_state);
                page.busy = None;
                match result {
                    Ok(outcome) => {
                        page.setup_open = false;
                        page.remove_open = false;
                        page.outcome = Some((name, outcome));
                    }
                    Err(error) => page.error = Some(format!("{name}: {error}")),
                }
            }
            read(&thread_ctx, &thread_state);
            thread_ctx.request_repaint();
        });
    if spawned.is_err() {
        lock(state).busy = None;
    }
}

fn action_label(action: &ExecAccountsAction) -> &'static str {
    match action {
        ExecAccountsAction::Setup { .. } => "Kontodienst einrichten",
        ExecAccountsAction::Allow { .. } => "Konto freigeben",
        ExecAccountsAction::Disallow { .. } => "Freigabe entziehen",
        ExecAccountsAction::Update => "Kontodienst aktualisieren",
        ExecAccountsAction::Remove { .. } => "Kontodienst entfernen",
    }
}
