//! `se screen portal forget [--json]` (SCR-M5, Linux): deletes the stored
//! Wayland screen grant of this user (the restore tokens in
//! `screen-portal-tokens.json`), so the next screen share asks in the system
//! dialog again. Running sessions go on. Text: `portal<TAB>forgotten<TAB>
//! screencast,remote_desktop` (or `none`); JSON `{"forgotten":["screencast",
//! "remote_desktop"]}` (an empty list when nothing was stored). Exit 0. There
//! is no MCP tool for it: a user's local grant is not for an agent to delete.
use clap::{Args, Subcommand};
use serde_json::{json, Value};

use crate::screen::os::portal_tokens::{self, TokenKind};

#[derive(Args, Debug, PartialEq, Eq)]
pub(super) struct PortalArgs {
    #[command(subcommand)]
    command: PortalCommand,
}

#[derive(Debug, PartialEq, Eq, Subcommand)]
enum PortalCommand {
    #[command(about = "Forget the stored screen grant; the next share asks again")]
    Forget {
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
}

/// The JSON and the text line of a `forget` that removed `removed`.
pub(super) fn forget_output(removed: &[TokenKind]) -> (Value, String) {
    let keys: Vec<&str> = removed.iter().map(|kind| kind.key()).collect();
    let listed = if keys.is_empty() {
        "none".to_string()
    } else {
        keys.join(",")
    };
    (
        json!({ "forgotten": keys }),
        format!("portal\tforgotten\t{listed}\n"),
    )
}

pub(super) fn run(args: PortalArgs) -> Result<i32, String> {
    match args.command {
        PortalCommand::Forget { json } => {
            let removed = portal_tokens::forget()
                .map_err(|error| format!("Bildschirmfreigabe nicht vergessen: {error}"))?;
            let (value, text) = forget_output(&removed);
            super::output::print(json, &value, || text);
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Wrapper {
        #[command(subcommand)]
        command: super::super::ScreenCommand,
    }

    #[test]
    fn screen_task_cli_portal_forget_json() {
        let (value, text) = forget_output(&[TokenKind::ScreenCast, TokenKind::RemoteDesktop]);
        assert_eq!(
            value,
            json!({"forgotten": ["screencast", "remote_desktop"]})
        );
        assert_eq!(text, "portal\tforgotten\tscreencast,remote_desktop\n");
        let (empty, none) = forget_output(&[]);
        assert_eq!(empty, json!({"forgotten": []}));
        assert_eq!(none, "portal\tforgotten\tnone\n");
        // The command line: `se screen portal forget --json`.
        let parsed = Wrapper::try_parse_from(["screen", "portal", "forget", "--json"]).unwrap();
        match parsed.command {
            super::super::ScreenCommand::Portal(args) => {
                assert_eq!(args.command, PortalCommand::Forget { json: true });
            }
            _ => panic!("not the portal command"),
        }
        assert!(Wrapper::try_parse_from(["screen", "portal"]).is_err());
    }
}
