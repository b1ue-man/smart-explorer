//! `se integration`: the settings of "Einstellungen → Integration" that are
//! not the background worker (`se background`) or the AI agent plugin
//! (`se plugin`): the Explorer context-menu entry "In Smart Explorer öffnen"
//! (Windows). The entry starts the installed app next to `se`.
use clap::{Args, Subcommand, ValueEnum};

const INTEGRATION_HELP: &str = "\
File manager integration of the desktop app (\"Settings → Integration\").
The Explorer context-menu entry exists only on Windows; it opens folders,
drives and the empty area of a window in the Smart Explorer app installed next
to this se (Windows 11: under \"Show more options\").

Examples:
  se integration context-menu
  se integration context-menu on
  se integration context-menu off";

#[derive(Args)]
#[command(long_about = INTEGRATION_HELP)]
pub(super) struct IntegrationArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(
        about = "Show or set the Explorer context-menu entry \"In Smart Explorer öffnen\" (Windows)"
    )]
    ContextMenu {
        #[arg(
            value_enum,
            help = "on or off; without a value the current state is shown"
        )]
        state: Option<Switch>,
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Switch {
    On,
    Off,
}

pub(super) fn run(args: IntegrationArgs) -> Result<i32, String> {
    match args.command {
        Command::ContextMenu { state, json } => context_menu(state, json),
    }
}

fn context_menu(state: Option<Switch>, json: bool) -> Result<i32, String> {
    let Some(current) = super::os::context_menu_enabled() else {
        if state.is_some() {
            return Err("the Explorer context-menu entry exists only on Windows".into());
        }
        return print(None, json);
    };
    let Some(wanted) = state.map(|state| state == Switch::On) else {
        return print(Some(current), json);
    };
    let app = match crate::updater::detect_installation()? {
        crate::updater::Installation::Desktop { app, .. } => app,
        crate::updater::Installation::TerminalOnly { .. } if !wanted => std::path::PathBuf::new(),
        crate::updater::Installation::TerminalOnly { .. } => {
            return Err(
                "no Smart Explorer app is installed next to this se; the entry would have nothing to open"
                    .into(),
            )
        }
    };
    super::os::set_context_menu(wanted, &app)?;
    print(super::os::context_menu_enabled(), json)
}

fn print(enabled: Option<bool>, json: bool) -> Result<i32, String> {
    if json {
        let value = serde_json::json!({
            "context_menu_supported": enabled.is_some(),
            "context_menu": enabled,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "context_menu\t{}",
            match enabled {
                Some(true) => "on",
                Some(false) => "off",
                None => "not available on this platform",
            }
        );
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::super::{Cli, Command as RootCommand};

    #[test]
    fn luecken_task_integration_cli_context_menu_states() {
        let commands: [&[&str]; 3] = [
            &["se", "integration", "context-menu"],
            &["se", "integration", "context-menu", "on"],
            &["se", "integration", "context-menu", "off", "--json"],
        ];
        for arguments in commands {
            let cli = Cli::try_parse_from(arguments).expect("parses");
            assert!(matches!(cli.command, RootCommand::Integration(_)));
        }
        assert!(Cli::try_parse_from(["se", "integration", "context-menu", "maybe"]).is_err());
        assert!(Cli::try_parse_from(["se", "integration"]).is_err());
    }
}
