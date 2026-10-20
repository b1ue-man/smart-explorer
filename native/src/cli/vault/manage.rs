//! `se vault list/show/detect/add/create/remove/password/forget-password/
//! settings` (vault plan C7.3, U26-11): the management of the vault list the
//! GUI shows in its sidebar. The commands that need the keys (unlock, lock)
//! are in `unlock.rs`. Passwords come only from standard input; they never
//! stand in an argument, a log or an error text.
use clap::Args;
use clap_complete::ArgValueCandidates;
use serde_json::{json, Value};

use crate::vault::{ManageError, OsSecretStore, PasswordError, RegistryStore, SettingsChange};

use super::complete::site_candidates;
use super::output::{clean, context, key_lines, pretty, site_of, site_value};

#[derive(Args)]
pub(super) struct SiteArg {
    #[arg(
        value_name = "SITE",
        help = "Vault ID, ID prefix (at least 4 digits) or label from `se vault list`",
        add = ArgValueCandidates::new(site_candidates)
    )]
    pub(super) site: String,
}

#[derive(Args)]
pub(super) struct SiteJsonArgs {
    #[command(flatten)]
    pub(super) site: SiteArg,
    #[arg(long, help = "Print machine-readable JSON")]
    pub(super) json: bool,
}

#[derive(Args)]
pub(super) struct DetectArgs {
    #[arg(
        value_name = "LOCATION",
        help = "Folder, saved remote (@label:/path) or locator"
    )]
    location: String,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
pub(super) struct AddArgs {
    #[arg(value_name = "LOCATION", help = "Folder of the vault")]
    location: String,
    #[arg(long, help = "Name shown in lists (default: the last folder name)")]
    label: Option<String>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
pub(super) struct CreateArgs {
    #[arg(value_name = "LOCATION", help = "New or empty folder for the vault")]
    location: String,
    #[arg(long, help = "Name shown in lists (default: the last folder name)")]
    label: Option<String>,
    #[arg(
        long,
        value_name = "N",
        value_parser = clap::value_parser!(u32).range(36..=220),
        help = "Shorten encrypted names longer than N characters (36 to 220, default 220)"
    )]
    shortening_threshold: Option<u32>,
    #[arg(
        long,
        help = "Read the new password from standard input (at least 8 characters)"
    )]
    password_stdin: bool,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
pub(super) struct PasswordArgs {
    #[command(flatten)]
    site: SiteArg,
    #[arg(
        long,
        help = "Read from standard input: line 1 the old password, line 2 the new one"
    )]
    stdin: bool,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
pub(super) struct SettingsArgs {
    #[command(flatten)]
    site: SiteArg,
    #[arg(long, help = "New name of the vault")]
    label: Option<String>,
    #[arg(
        long,
        value_name = "MINUTES|off",
        help = "Lock after this many idle minutes, or off"
    )]
    auto_lock_idle_min: Option<String>,
    #[arg(long, help = "Lock when the computer sleeps (true/false)")]
    lock_on_sleep: Option<bool>,
    #[arg(long, help = "Lock with the screen lock (true/false)")]
    lock_on_screen_lock: Option<bool>,
    #[arg(
        long,
        value_name = "0|1|2|5|10|off",
        help = "Android: lock after this many minutes in the background (0 = at once), or off"
    )]
    lock_on_background_min: Option<String>,
    #[arg(
        long,
        help = "Remember the password (only off here; a password is remembered by `unlock --remember true`)"
    )]
    remember_password: Option<bool>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

/// A location as the GUI stores it: `@label:/path` of a saved remote
/// becomes its endpoint, a relative local path is made absolute.
pub(super) fn location_of(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Der Ort darf nicht leer sein".into());
    }
    if let Some((connection, path)) = crate::cli::target::saved_shorthand(text)? {
        if !connection.protocol.is_url() {
            return Err(format!(
                "{text}: bitte den Netzwerkpfad selbst angeben (\\\\Server\\Freigabe\\…)"
            ));
        }
        return Ok(crate::connect::remote_endpoint(&connection, &path));
    }
    let remote = text.contains("://") || text.starts_with("\\\\") || text.starts_with("//");
    if remote || std::path::Path::new(text).is_absolute() {
        return Ok(text.to_string());
    }
    std::path::absolute(text)
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|error| format!("{text}: {error}"))
}

fn manage_error(error: ManageError) -> String {
    match error {
        ManageError::InUse(ids) => {
            // Name the jobs, not only their IDs.
            let jobs = crate::syncjobs::load_report()
                .map(|report| report.jobs)
                .unwrap_or_default();
            let named: Vec<String> = ids
                .iter()
                .map(|id| {
                    jobs.iter()
                        .find(|job| &job.id == id)
                        .map_or_else(|| id.clone(), |job| format!("{} ({id})", clean(&job.name)))
                })
                .collect();
            format!(
                "Tresor-Sync-Jobs nutzen diesen Tresorort: {}; zuerst die Jobs entfernen",
                named.join(", ")
            )
        }
        other => other.to_string(),
    }
}

pub(super) fn list(json: bool) -> Result<i32, String> {
    let load = RegistryStore::open_default()
        .load()
        .map_err(|error| format!("Tresorliste lesen: {error}"))?;
    for (file, reason) in &load.broken {
        eprintln!(
            "se: Tresorliste: {} nicht lesbar: {}",
            clean(file),
            clean(reason)
        );
    }
    let context = context();
    // The sites of one vault (one vault ID) are one group; a site without a
    // vault ID is a group of its own.
    let mut groups: Vec<(Option<String>, Vec<&crate::vault::SiteRecord>)> = Vec::new();
    for site in &load.sites {
        let known = (!site.vault_id.is_empty())
            .then(|| {
                groups
                    .iter()
                    .position(|(id, _)| id.as_deref() == Some(site.vault_id.as_str()))
            })
            .flatten();
        match known {
            Some(index) => groups[index].1.push(site),
            None => groups.push((
                (!site.vault_id.is_empty()).then(|| site.vault_id.clone()),
                vec![site],
            )),
        }
    }
    if json {
        let vaults: Vec<Value> = groups
            .iter()
            .map(|(id, sites)| {
                json!({
                    "vault_id": id,
                    "sites": sites.iter().map(|site| site_value(site, &context)).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", pretty(&json!({ "vaults": vaults }))?);
        return Ok(0);
    }
    if load.sites.is_empty() {
        println!("vaults\t0");
    }
    for (_, sites) in &groups {
        for site in sites {
            println!(
                "vault\t{}\t{}\t{}\t{}\t{}\t{}",
                site.id,
                clean(site.display_label()),
                site.kind.as_str(),
                context.state_of(&site.id),
                clean(&site.location),
                context
                    .mounted_of(&site.id)
                    .map_or_else(|| "-".to_string(), clean),
            );
        }
    }
    Ok(0)
}

pub(super) fn show(args: SiteJsonArgs) -> Result<i32, String> {
    let record = site_of(&args.site.site)?;
    print_site(&record, args.json)
}

fn print_site(record: &crate::vault::SiteRecord, json: bool) -> Result<i32, String> {
    let context = context();
    let mut value = site_value(record, &context);
    let recovery = crate::vault::recovery_key_info();
    if let Value::Object(fields) = &mut value {
        fields.insert(
            "shortening_threshold".into(),
            json!(context.threshold_of(&record.id)),
        );
        fields.insert("recovery_key_supported".into(), json!(recovery.supported));
        fields.insert("recovery_key_note".into(), json!(recovery.note));
    }
    if json {
        println!("{}", pretty(&value)?);
    } else {
        for line in key_lines(&value) {
            println!("{line}");
        }
    }
    Ok(0)
}

fn resolve_location(location: &str) -> Result<(crate::vfs::BackendHandle, String), String> {
    crate::connect::resolve_endpoint(location)
}

pub(super) fn detect(args: DetectArgs) -> Result<i32, String> {
    let location = location_of(&args.location)?;
    let detection =
        crate::vault::detect_location(&location, &resolve_location).map_err(manage_error)?;
    if args.json {
        println!(
            "{}",
            pretty(&json!({
                "is_vault": detection.is_vault,
                "format": detection.format,
                "key_source": detection.key_source,
                "vault_id": detection.vault_id,
            }))?
        );
    } else {
        println!(
            "detect\t{}\t{}\t{}\t{}",
            if detection.is_vault { "yes" } else { "no" },
            detection
                .format
                .map_or_else(|| "-".to_string(), |format| format.to_string()),
            detection.key_source.unwrap_or("-"),
            detection.vault_id.as_deref().unwrap_or("-"),
        );
        if let Some(problem) = &detection.problem {
            eprintln!("se: {}", clean(problem));
        }
    }
    Ok(0)
}

pub(super) fn add(args: AddArgs) -> Result<i32, String> {
    let location = location_of(&args.location)?;
    let record = crate::vault::add_site(
        &RegistryStore::open_default(),
        &location,
        args.label.as_deref(),
        &resolve_location,
    )
    .map_err(manage_error)?;
    if args.json {
        println!(
            "{}",
            pretty(&json!({
                "site": record.id,
                "label": record.display_label(),
                "vault_id": record.vault_id,
            }))?
        );
    } else {
        println!("added\t{}\t{}", record.id, clean(record.display_label()));
    }
    Ok(0)
}

pub(super) fn create(args: CreateArgs) -> Result<i32, String> {
    if !args.password_stdin {
        return Err("Passwort fehlt: --password-stdin".into());
    }
    let location = location_of(&args.location)?;
    let password = crate::cli::setup::read_stdin_secret()?;
    let record = crate::vault::create_site(
        &RegistryStore::open_default(),
        &location,
        args.label.as_deref(),
        &password,
        args.shortening_threshold
            .unwrap_or(crate::cryptomator::DEFAULT_SHORTENING_THRESHOLD),
        crate::vault::available_memory(),
        &resolve_location,
    )
    .map_err(manage_error)?;
    if args.json {
        println!(
            "{}",
            pretty(&json!({ "site": record.id, "vault_id": record.vault_id }))?
        );
    } else {
        println!("created\t{}\t{}", record.id, clean(record.display_label()));
    }
    eprintln!("{}", crate::vault::CREATE_RECOVERY_HINT);
    Ok(0)
}

pub(super) fn remove(args: SiteArg) -> Result<i32, String> {
    let record = site_of(&args.site)?;
    let removed = crate::vault::remove_entered(&record.id).map_err(manage_error)?;
    println!("removed\t{}", removed.id);
    Ok(0)
}

/// Old and new password from two lines of standard input.
pub(super) fn two_passwords(text: &str) -> Result<(String, String), String> {
    let missing = || "Passwort fehlt: Zeile 1 das alte, Zeile 2 das neue Passwort".to_string();
    let (old, rest) = text.split_once('\n').ok_or_else(missing)?;
    let new = rest.split('\n').next().unwrap_or_default();
    let old = old.strip_suffix('\r').unwrap_or(old);
    let new = new.strip_suffix('\r').unwrap_or(new);
    if new.is_empty() {
        return Err(missing());
    }
    Ok((old.to_string(), new.to_string()))
}

pub(super) fn password(args: PasswordArgs) -> Result<i32, String> {
    if !args.stdin {
        return Err("Passwort fehlt: --stdin (Zeile 1 das alte, Zeile 2 das neue Passwort)".into());
    }
    let record = site_of(&args.site.site)?;
    let (old, new) = two_passwords(&crate::cli::setup::read_stdin_secret()?)?;
    let changed = crate::vault::change_site_password(
        &RegistryStore::open_default(),
        &OsSecretStore,
        &record.id,
        &old,
        &new,
        crate::vault::available_memory(),
        &resolve_location,
    )
    .map_err(|error: PasswordError| {
        error
            .code()
            .and_then(crate::vault::error_text_for_code)
            .map_or_else(|| error.to_string(), str::to_string)
    })?;
    if let Some(problem) = &changed.remember_error {
        eprintln!("se: Neues Passwort nicht gemerkt: {}", clean(problem));
    }
    if args.json {
        println!(
            "{}",
            pretty(&json!({ "site": record.id, "backup": !changed.backup.is_empty() }))?
        );
    } else {
        println!("password\t{}\tchanged", record.id);
    }
    Ok(0)
}

pub(super) fn forget_password(args: SiteArg) -> Result<i32, String> {
    let record = site_of(&args.site)?;
    let change = SettingsChange {
        remember_password: Some(false),
        ..SettingsChange::default()
    };
    crate::vault::update_settings(
        &RegistryStore::open_default(),
        &record.id,
        &change,
        &OsSecretStore,
    )
    .map_err(manage_error)?;
    println!("forgotten\t{}", record.id);
    Ok(0)
}

/// `N` or `off` as the value of a timed lock.
pub(super) fn minutes(text: &str, option: &str) -> Result<Option<u32>, String> {
    if text.eq_ignore_ascii_case("off") {
        return Ok(None);
    }
    text.parse::<u32>()
        .map(Some)
        .map_err(|_| format!("{option}: eine Zahl oder off erwartet, nicht „{text}“"))
}

pub(super) fn settings(args: SettingsArgs) -> Result<i32, String> {
    let record = site_of(&args.site.site)?;
    let change = SettingsChange {
        label: args.label,
        auto_lock_idle_min: args
            .auto_lock_idle_min
            .as_deref()
            .map(|text| minutes(text, "--auto-lock-idle-min"))
            .transpose()?,
        lock_on_sleep: args.lock_on_sleep,
        lock_on_screen_lock: args.lock_on_screen_lock,
        lock_on_background_min: args
            .lock_on_background_min
            .as_deref()
            .map(|text| minutes(text, "--lock-on-background-min"))
            .transpose()?,
        remember_password: args.remember_password,
    };
    let record = if change == SettingsChange::default() {
        record
    } else {
        crate::vault::update_settings(
            &RegistryStore::open_default(),
            &record.id,
            &change,
            &OsSecretStore,
        )
        .map_err(manage_error)?
    };
    print_site(&record, args.json)
}
