//! `se connections gdrive`: the Google Drive account of the GUI (desktop
//! "Einstellungen → Verbindungen → Google Drive", Android "Google Drive"):
//! the OAuth client of the user's own Google project, the browser sign-in
//! with its loopback redirect on this computer, sign-out and status. The
//! values are the GUI's (`cloud::save_config`, the stored refresh token).
use clap::{Args, Subcommand};
use serde_json::json;

use crate::cloud::{ClientConfig, Provider};

const GDRIVE_HELP: &str = "\
Google Drive uses an OAuth client of your own Google Cloud project (type
\"Desktop app\", see docs/CLOUD_SETUP.md). `sign-in` saves the client like the
app's \"Mit Google verbinden\", opens the browser on this computer and waits up
to five minutes for the redirect to 127.0.0.1. Over SSH, open the printed
address in a browser on the same computer (or forward the shown port).

Examples:
  se connections gdrive status --json
  se connections gdrive sign-in --client-id 123-abc.apps.googleusercontent.com --client-secret-stdin
  se connections gdrive configure --client-id 123-abc.apps.googleusercontent.com
  se connections gdrive sign-out
  se ls gdrive:///";

#[derive(Args)]
#[command(long_about = GDRIVE_HELP)]
pub(super) struct GdriveArgs {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Show whether a client is configured and an account is signed in")]
    Status {
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Save the OAuth client id and secret without signing in")]
    Configure(ClientArgs),
    #[command(about = "Save the client (when given) and sign in through the browser")]
    SignIn(SignInArgs),
    #[command(about = "Disconnect Google Drive (removes the stored sign-in)")]
    SignOut,
}

#[derive(Args)]
struct ClientArgs {
    #[arg(long, help = "OAuth client id (…apps.googleusercontent.com)")]
    client_id: String,
    #[command(flatten)]
    secret: SecretArgs,
}

#[derive(Args)]
struct SignInArgs {
    #[arg(long, help = "OAuth client id to save first (default: the saved one)")]
    client_id: Option<String>,
    #[command(flatten)]
    secret: SecretArgs,
}

#[derive(Args)]
struct SecretArgs {
    #[arg(
        long,
        conflicts_with_all = ["client_secret_stdin", "clear_client_secret"],
        help = "OAuth client secret (default: keep the saved one)"
    )]
    client_secret: Option<String>,
    #[arg(
        long,
        conflicts_with = "clear_client_secret",
        help = "Read the client secret from stdin"
    )]
    client_secret_stdin: bool,
    #[arg(long, help = "Remove the saved client secret (pure PKCE clients)")]
    clear_client_secret: bool,
}

impl SecretArgs {
    /// `None` keeps the saved secret, like the Android `gdrive.configure`.
    fn read(&self) -> Result<Option<String>, String> {
        if self.clear_client_secret {
            return Ok(Some(String::new()));
        }
        if self.client_secret_stdin {
            return super::setup::read_stdin_secret().map(Some);
        }
        Ok(self.client_secret.clone())
    }
}

pub(super) fn run(args: GdriveArgs) -> Result<i32, String> {
    match args.command.unwrap_or(Command::Status { json: false }) {
        Command::Status { json } => {
            let value = status_value();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
                );
            } else {
                println!(
                    "client\t{}",
                    value["client_id"].as_str().unwrap_or("not configured")
                );
                println!(
                    "signed_in\t{}",
                    if value["signed_in"] == true {
                        "yes"
                    } else {
                        "no"
                    }
                );
            }
            Ok(0)
        }
        Command::Configure(args) => {
            let secret = args.secret.read()?;
            save_client(Some(args.client_id), secret)?;
            println!("Saved the Google Drive client; sign in with `se connections gdrive sign-in`");
            Ok(0)
        }
        Command::SignIn(args) => {
            let secret = args.secret.read()?;
            if args.client_id.is_some() || secret.is_some() {
                save_client(args.client_id, secret)?;
            }
            sign_in()
        }
        Command::SignOut => {
            crate::cloud::disconnect(Provider::GDrive)?;
            println!("Google Drive disconnected");
            Ok(0)
        }
    }
}

/// The document of `status --json`: the fields of the Android `gdrive.status`.
pub(super) fn status_value() -> serde_json::Value {
    status_json(
        &crate::cloud::load_config(Provider::GDrive),
        crate::cloud::is_connected(Provider::GDrive),
    )
}

fn status_json(config: &ClientConfig, signed_in: bool) -> serde_json::Value {
    let client_id = config.client_id.trim().to_string();
    json!({
        "client_configured": !client_id.is_empty(),
        "client_id": (!client_id.is_empty()).then_some(client_id),
        "signed_in": signed_in,
    })
}

/// A missing id or secret keeps the saved one; an empty secret clears it.
fn save_client(client_id: Option<String>, secret: Option<String>) -> Result<(), String> {
    let saved = crate::cloud::load_config_checked(Provider::GDrive)
        .map_err(|error| format!("read the Google Drive client: {error}"))?;
    let config = ClientConfig {
        client_id: client_id.map_or(saved.client_id, |id| id.trim().to_string()),
        client_secret: secret.map_or(saved.client_secret, |secret| secret.trim().to_string()),
    };
    if config.client_id.is_empty() {
        return Err("the OAuth client id is missing: pass --client-id".into());
    }
    if [&config.client_id, &config.client_secret]
        .iter()
        .any(|value| value.chars().any(char::is_control))
    {
        return Err("client id or secret contains control characters".into());
    }
    crate::cloud::save_config(Provider::GDrive, &config)
        .map_err(|error| format!("save the Google Drive client: {error}"))
}

fn sign_in() -> Result<i32, String> {
    if crate::cloud::load_config(Provider::GDrive)
        .client_id
        .trim()
        .is_empty()
    {
        return Err("no OAuth client id is configured: pass --client-id".into());
    }
    // The terminal shows the address and then opens the browser itself.
    crate::cloud::set_url_opener(Box::new(|url: &str| {
        eprintln!("Open this address in a browser on this computer to sign in:\n{url}");
        if let Err(error) = crate::cloud::open_platform_url(url) {
            eprintln!("se: the browser could not be opened: {error}");
        }
    }));
    eprintln!("Waiting up to five minutes for the Google sign-in …");
    crate::cloud::authorize(Provider::GDrive)
        .map_err(|error| format!("Google sign-in: {error}"))?;
    println!("Google Drive connected; browse it with `se ls gdrive:///`");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::super::super::{Cli, Command as RootCommand};

    fn parses(arguments: &[&str]) -> bool {
        let mut all = vec!["se", "connections", "gdrive"];
        all.extend_from_slice(arguments);
        Cli::try_parse_from(all).is_ok_and(|cli| matches!(cli.command, RootCommand::Connections(_)))
    }

    #[test]
    fn luecken_task_gdrive_cli_has_every_gui_action() {
        assert!(parses(&[]));
        assert!(parses(&["status", "--json"]));
        assert!(parses(&[
            "configure",
            "--client-id",
            "x.apps.googleusercontent.com"
        ]));
        assert!(parses(&[
            "sign-in",
            "--client-id",
            "x.apps.googleusercontent.com",
            "--client-secret-stdin"
        ]));
        assert!(parses(&["sign-in"]));
        assert!(parses(&["sign-out"]));
        assert!(!parses(&["configure"]));
        assert!(!parses(&[
            "configure",
            "--client-id",
            "x",
            "--client-secret",
            "s",
            "--clear-client-secret"
        ]));
    }

    #[test]
    fn luecken_task_gdrive_cli_status_has_the_android_fields() {
        let config = crate::cloud::ClientConfig {
            client_id: " id.apps.googleusercontent.com ".into(),
            client_secret: "secret".into(),
        };
        let value = super::status_json(&config, true);
        assert_eq!(value["client_configured"], true);
        assert_eq!(value["client_id"], "id.apps.googleusercontent.com");
        assert_eq!(value["signed_in"], true);
        // The secret never appears in the status.
        assert!(!value.to_string().contains("secret"));
        let empty = crate::cloud::ClientConfig {
            client_id: String::new(),
            client_secret: String::new(),
        };
        let value = super::status_json(&empty, false);
        assert_eq!(value["client_configured"], false);
        assert!(value["client_id"].is_null());
    }
}
