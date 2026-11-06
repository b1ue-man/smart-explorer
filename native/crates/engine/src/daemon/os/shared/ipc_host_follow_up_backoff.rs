//! Pacing of the waiting Exec follow-ups of the share worker (A11 of plan
//! 2026-11-06-akku-rechte, `exec_grant_journal_follow_ups.rs`), kept in the
//! reload gate because every attempt is a complete reload.

/// Failed attempts of the waiting Exec follow-ups in a row and the profiles
/// the last failure saw (`exec_grant_journal_follow_ups.rs`). A change of
/// the profiles beyond runtime data (rights, contacts, rooms, waiting
/// changes) starts the backoff from the beginning; presence and routes,
/// which change with every presence event, do not.
#[derive(Debug, Default)]
pub(in crate::daemon) struct FollowUpBackoff {
    failures: u32,
    seen: Option<crate::share::ShareProfiles>,
}

impl FollowUpBackoff {
    /// A follow-up attempt failed under `profiles`; returns how many
    /// attempts failed in a row before this one.
    pub(in crate::daemon) fn failed(&mut self, profiles: &crate::share::ShareProfiles) -> u32 {
        let before = self.failures;
        self.failures = before.saturating_add(1);
        self.seen = Some(profiles.clone());
        before
    }

    /// The follow-ups were applied, or none waits any more.
    pub(in crate::daemon) fn settled(&mut self) {
        *self = Self::default();
    }

    /// True (and the backoff starts over) when `profiles` differ from the
    /// last failure's beyond runtime data.
    pub(in crate::daemon) fn profiles_changed(
        &mut self,
        profiles: &crate::share::ShareProfiles,
    ) -> bool {
        let Some(seen) = self.seen.as_mut() else {
            return false;
        };
        let same_digest = matches!(
            profiles.storage_revision,
            crate::share::ProfileRevision::Digest(_)
        ) && seen.storage_revision == profiles.storage_revision;
        if same_digest {
            return false;
        }
        if seen.exec_follow_ups != profiles.exec_follow_ups
            || crate::share::profiles_differ_beyond_runtime(seen, profiles)
        {
            self.settled();
            return true;
        }
        // Runtime data only: compare the next change with these profiles.
        *seen = profiles.clone();
        false
    }
}
