//! Find offers and pair through the daemon's shared discovery/trust runtime.
use clap::{Args, Subcommand};

use super::{discover_input, discover_output};

#[derive(Args)]
#[command(long_about = "Find peers and rooms published with `se share discoverable`.\n\
Pair using the OFFER discovery ID printed by list and exactly one PIN source.\n\
--pin-stdin reads all UTF-8 bytes up to EOF, including spaces and line endings.\n\
--pin-prompt requires an interactive terminal and hides the typed PIN.\n\
Pairing waits for the shared daemon's result; Ctrl+C cancels only this exchange.\n\
Exports are shared back only with the explicit --share-back option.")]
pub(super) struct DiscoverArgs {
    #[command(subcommand)]
    command: DiscoverCommand,
}

#[derive(Subcommand)]
enum DiscoverCommand {
    #[command(about = "List currently discoverable offers without consuming GUI events")]
    List {
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Pair an offer using its PIN; wait for completion or cancel with Ctrl+C")]
    Pair(PairArgs),
}

#[derive(Args)]
#[command(group(clap::ArgGroup::new("pin_source").required(true).args(["pin_stdin", "pin_prompt"])))]
struct PairArgs {
    #[arg(value_name = "OFFER", help = "Stable discovery ID from `se share discover list`")]
    offer: String,
    #[arg(long, help = "Read exact UTF-8 PIN bytes until stdin reaches EOF")]
    pin_stdin: bool,
    #[arg(long, help = "Read the PIN without echo from an interactive terminal")]
    pin_prompt: bool,
    #[arg(long, help = "Explicitly open this device's exports to the paired direct device")]
    share_back: bool,
    #[arg(long)]
    json: bool,
}

pub(super) fn run(args: DiscoverArgs) -> Result<i32, String> {
    Ok(match args.command {
        DiscoverCommand::List { json } => match crate::daemon::list_discoveries() {
            Ok(offers) => { discover_output::list(&offers, json); 0 }
            Err(error) => discover_output::failure(None, None, error, json),
        },
        DiscoverCommand::Pair(args) => pair(args),
    })
}

fn pair(args: PairArgs) -> i32 {
    let pin = match discover_input::read_pin(args.pin_stdin, args.pin_prompt) {
        Ok(pin) => pin,
        Err(message) => return discover_output::failure_text(Some(&args.offer), None, "pin_input", message, args.json),
    };
    let session = match crate::daemon::start_discovery_pairing(args.offer.clone(), pin, args.share_back) {
        Ok(session) => session,
        Err(error) => return discover_output::failure(Some(&args.offer), None, error, args.json),
    };
    let exchange_id = session.exchange_id().to_string();
    if !args.json { eprintln!("Pairing in progress. Ctrl+C cancels this exchange."); }
    match session.wait() {
        Ok(reply) => discover_output::finished(&reply, args.json),
        Err(error) => discover_output::failure(Some(&args.offer), Some(&exchange_id), error, args.json),
    }
}
