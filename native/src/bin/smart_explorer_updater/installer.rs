//! Installer bridge into the existing apply/replace/generation transaction.
use super::args::{ApplyArgs,InstallMode};
use std::path::PathBuf;

pub(super) fn run(raw:&[String])->Result<(),String> {
    let allowed=["--root","--app-staged","--app-sha256","--cli-staged","--cli-sha256","--launcher-staged","--launcher-sha256","--helper-sha256","--version"];
    let mut values=std::collections::BTreeMap::new();let mut cli_only=false;let mut no_gui=false;let mut index=2;
    while index<raw.len() {
        let key=raw[index].as_str();
        if matches!(key,"--cli-only"|"--no-gui-start") {if key=="--cli-only" {if cli_only {return Err("duplicate --cli-only".into());}cli_only=true;} else {if no_gui {return Err("duplicate --no-gui-start".into());}no_gui=true;} index+=1;continue;}
        if !allowed.contains(&key)||index+1>=raw.len()||values.insert(key,raw[index+1].clone()).is_some() {return Err(format!("unknown, duplicate or incomplete installer argument: {key}"));}
        index+=2;
    }
    let get=|key:&str|values.get(key).cloned().ok_or_else(||format!("required installer argument missing: {key}"));
    let root=std::fs::canonicalize(PathBuf::from(get("--root")?)).map_err(|e|e.to_string())?;
    smart_explorer::mcp_launcher::validate_installation_root(&root)?;
    let helper=std::env::current_exe().map_err(|e|e.to_string())?;
    let helper_sha256=super::hash::normalize_sha256(&get("--helper-sha256")?)?;
    super::hash::verify_sha256(&helper,&helper_sha256)?;
    let cli_staged=PathBuf::from(get("--cli-staged")?);let cli_sha256=super::hash::normalize_sha256(&get("--cli-sha256")?)?;
    let launcher_staged=PathBuf::from(get("--launcher-staged")?);let launcher_sha256=super::hash::normalize_sha256(&get("--launcher-sha256")?)?;
    super::hash::verify_sha256(&cli_staged,&cli_sha256)?;super::hash::verify_sha256(&launcher_staged,&launcher_sha256)?;
    let (app_name,cli_name,helper_name)=installed_names();
    let target=root.join(app_name);let cli_target=root.join(cli_name);
    if cli_only && target.exists() {
        let expected=super::hash::normalize_sha256(&get("--app-sha256").map_err(|_|"installation_type_conflict: --cli-only beside a desktop app requires its expected feed SHA-256")?)?;
        smart_explorer::mcp_launcher::validate_installation_path(&root,&target)?;
        super::hash::verify_sha256(&target,&expected).map_err(|_|"installation_type_conflict: installed desktop app differs from the expected feed SHA-256")?;
    }
    let first=!target.exists();
    let staged=if cli_only {root.join(".unused-app-install-payload")}else{PathBuf::from(get("--app-staged")?)};
    let staged_sha256=if cli_only {if target.exists() {super::hash::normalize_sha256(&get("--app-sha256")?)?}else{cli_sha256.clone()}}else{super::hash::normalize_sha256(&get("--app-sha256")?)?};
    if !cli_only {super::hash::verify_sha256(&staged,&staged_sha256)?;}
    let version=get("--version")?;
    if !smart_explorer::mcp_launcher::core::manifest::is_id(&version) {return Err("installer product version invalid".into());}
    let state=root.join("mcp");std::fs::create_dir_all(&state).map_err(|e|e.to_string())?;
    let archive_dir=root.join(".versions");std::fs::create_dir_all(&archive_dir).map_err(|e|e.to_string())?;
    smart_explorer::mcp_launcher::validate_installation_path(&root,&state)?;
    smart_explorer::mcp_launcher::validate_installation_path(&root,&archive_dir)?;
    let target_sha256=if target.exists() {super::hash::sha256_file(&target)?}else{staged_sha256.clone()};
    let install_mode=if cli_only {InstallMode::CliOnly}else if no_gui||first||is_elevated_context() {InstallMode::Verified}else{InstallMode::GuiAck};
    let args=ApplyArgs {install_mode,target,target_sha256,staged,staged_sha256,helper_target:root.join(helper_name),helper_sha256,cli_staged,cli_target,cli_sha256,launcher_staged:Some(launcher_staged),launcher_sha256:Some(launcher_sha256),archive:archive_dir.join(format!("previous-{version}-{app_name}")),parent_pid:0,version:version.clone(),last_applied:state.join("install-last-applied.txt"),error_file:state.join("install-error.txt"),manifest:state.join("installer-staging.json"),pin_file:state.join("installer-pin.txt")};
    super::apply_update(args).map_err(|e|e.message)?;
    println!("{}",serde_json::json!({"version":version,"status":"installed","completion":if install_mode==InstallMode::GuiAck {"gui_startup_ack"}else{"verified_installation"},"gui_started":install_mode==InstallMode::GuiAck,"user_mcp_started":false,"profile_refresh":if is_elevated_context() {"deferred_original_user_start"}else{"attempted_with_verified_installed_generator"},"note":if install_mode==InstallMode::GuiAck {"The normal app startup was acknowledged."}else{"Installed payload hashes and CLI product version were verified. No GUI or user MCP process was started; the next user start negotiates its daemon contract."}}));
    Ok(())
}
#[cfg(windows)]
fn installed_names()->(&'static str,&'static str,&'static str) {("Smart Explorer.exe","se.exe","Smart Explorer Updater.exe")}
#[cfg(not(windows))]
fn installed_names()->(&'static str,&'static str,&'static str) {("smart_explorer","se","smart_explorer_updater")}
pub(super) fn is_elevated_context()->bool {
    #[cfg(not(windows))] {unsafe {libc::geteuid()==0}}
    #[cfg(windows)] {
        use windows_sys::Win32::{Security::{GetTokenInformation,TokenElevation,TOKEN_ELEVATION,TOKEN_QUERY},System::Threading::{GetCurrentProcess,OpenProcessToken},Foundation::CloseHandle};
        unsafe {
            let mut token=std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&mut token)==0 {return true;}
            let mut elevation:TOKEN_ELEVATION=std::mem::zeroed();let mut length=0;
            let ok=GetTokenInformation(token,TokenElevation,&mut elevation as *mut _ as _,std::mem::size_of_val(&elevation) as u32,&mut length);
            CloseHandle(token);ok==0 || elevation.TokenIsElevated!=0
        }
    }
}
pub(super) fn verify_completion(args:&ApplyArgs)->Result<(),String> {
    let root=args.cli_target.parent().ok_or("installed CLI parent missing")?;
    if args.install_mode==InstallMode::CliOnly && args.target.exists() {super::hash::verify_sha256(&args.target,&args.staged_sha256)?;}
    if args.launcher_staged.is_some() {smart_explorer::mcp_launcher::verify_installed_product(root,&args.cli_target,&args.cli_sha256,&args.version)}
    else {smart_explorer::mcp_launcher::verify_legacy_product(root,&args.cli_target,&args.cli_sha256,&args.version)}
}

/// A machine installer never writes a user's profiles from its admin token.
pub(super) fn refresh_after_commit(args:&ApplyArgs) {
    if is_elevated_context() {
        super::logging::append_log("MCP profile refresh deferred to the next original-user MCP start: elevated installer context");
        return;
    }
    let Some(root)=args.cli_target.parent() else {return;};
    if root.join("mcp/current.v1.json").exists() {
        if let Err(error)=smart_explorer::mcp_launcher::refresh_installed_profile(root) {
            super::logging::append_log(&format!("MCP profile refresh after committed install: {error}"));
        }
    }
}
