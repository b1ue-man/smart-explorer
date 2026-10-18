//! The screen rows of the capability catalog (contract V7, E point 3):
//! „Bildschirm ansehen“ and „Bildschirm steuern“, only on hosts with a
//! graphical session, so a server, a root worker or a phone before SCR-M8
//! offers no right it cannot honour. `access_catalog::capability_catalog`
//! calls `register_screen_entries` after the Exec rows; the access dialogs,
//! `se share grants access` (`--screen-view`, `--screen-control`) and the
//! Android bridge (`screenView`, `screenControl`) take the rows from there.
use crate::share::access_catalog::{CapabilityEntry, HostFacts};
use crate::share::access_grant::Capability;

/// Appends the screen rows when `facts` has a graphical session.
pub(crate) fn register_screen_entries(entries: &mut Vec<CapabilityEntry>, facts: &HostFacts) {
    if !facts.graphical_session {
        return;
    }
    entries.push(CapabilityEntry {
        capability: Capability::ScreenView,
        label: "Bildschirm ansehen",
        description: "Den Bildschirm und die laufenden Fenster dieses Geräts sehen",
        cli_option: "screen-view",
        bridge_key: "screenView",
        uses_account: false,
    });
    entries.push(CapabilityEntry {
        capability: Capability::ScreenControl,
        label: "Bildschirm steuern",
        description:
            "Maus, Tastatur und Text dieses Geräts bedienen (setzt „Bildschirm ansehen“ voraus)",
        cli_option: "screen-control",
        bridge_key: "screenControl",
        uses_account: false,
    });
}

#[cfg(test)]
mod tests {
    use crate::share::access_catalog::{capability_catalog, HelperState, HelperStatus, HostFacts};
    use crate::share::access_grant::Capability;

    fn facts(graphical_session: bool) -> HostFacts {
        HostFacts {
            graphical_session,
            account_helper: HelperStatus {
                state: HelperState::NotNeeded,
                version: None,
                detail: None,
            },
        }
    }

    #[test]
    fn screen_task_catalog_rows_only_with_graphical_session() {
        let without: Vec<Capability> = capability_catalog(&facts(false))
            .into_iter()
            .map(|entry| entry.capability)
            .collect();
        assert_eq!(without, Capability::EXEC);
        let with = capability_catalog(&facts(true));
        let capabilities: Vec<Capability> = with.iter().map(|entry| entry.capability).collect();
        assert_eq!(capabilities, Capability::ALL, "Exec first, then the screen");
        let view = &with[2];
        assert_eq!(
            (
                view.label,
                view.cli_option,
                view.bridge_key,
                view.uses_account
            ),
            ("Bildschirm ansehen", "screen-view", "screenView", false)
        );
        let control = &with[3];
        assert_eq!(
            (
                control.label,
                control.cli_option,
                control.bridge_key,
                control.uses_account
            ),
            (
                "Bildschirm steuern",
                "screen-control",
                "screenControl",
                false
            )
        );
        // The CLI options are the keys of the stored form with dashes.
        for entry in &with[2..] {
            assert_eq!(entry.cli_option.replace('-', "_"), entry.capability.key());
        }
    }
}
