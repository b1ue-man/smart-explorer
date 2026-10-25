//! A launcher supervises exactly one immutable runtime without interpreting MCP.
use crate::mcp_launcher::{os,Installation,RuntimeIdentity,GenerationChild};
use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc,atomic::{AtomicBool,Ordering}};

pub fn run_launcher()->Result<i32,String> {
    let args=std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.as_slice()==[std::ffi::OsString::from("--launcher-contract")] {
        println!("{}",serde_json::to_string(&crate::mcp_launcher::core::capability::LauncherProbe::current()).map_err(|e|e.to_string())?);return Ok(0);
    }
    if args.first().is_some_and(|a|a=="--provision") {
        return crate::mcp_launcher::installer_cli::provision(&args[1..]);
    }
    if args.as_slice()==[std::ffi::OsString::from("--version")] {
        println!("se-mcp-launcher {} protocol {}",env!("CARGO_PKG_VERSION"),crate::mcp_launcher::core::manifest::LAUNCHER_PROTOCOL);
        return Ok(0);
    }
    if args.len()>1 || args.first().is_some_and(|a|a!="mcp") {
        return Err("launcher accepts only mcp or --version".into());
    }
    let image=std::fs::canonicalize(std::env::current_exe().map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let installation=Installation::for_image(&image)?.ok_or("launcher_installation_missing")?;
    let hash=image.parent().and_then(Path::file_name).and_then(|s|s.to_str()).ok_or("launcher_hash_path_missing")?;
    let expected=installation.root.join("mcp/launchers").join(hash).join(os::LAUNCHER_NAME);
    if image!=expected || !installation.manifest.allowed_launcher_hashes.iter().any(|allowed|allowed==hash) {
        return Err("launcher_revoked_or_migration_required".into());
    }
    let _own_image=os::open_verified(&installation.root,&image,hash)?;
    launch(&installation)
}

pub fn launch(installation:&Installation)->Result<i32,String> {
    let control=installation.lock("control.lock",false)?;
    crate::mcp_launcher::admission::check_state(installation)?;
    let runtime=RuntimeIdentity {
        installation:installation.clone(),
        contract:crate::mcp_launcher::RuntimeContract {
            runtime_sha256:installation.manifest.runtime_sha256.clone(),
            ipc_contract_id:installation.manifest.ipc_contract_id.clone(),
            shared_state_contract_id:installation.manifest.shared_state_contract_id.clone(),
        },
        executable:installation.runtime_path(),
    };
    let lifetime=runtime.lifetime()?;
    let image=os::open_verified(&installation.root,&runtime.executable,&runtime.contract.runtime_sha256)?;
    let starter=std::env::current_exe().map_err(|e|e.to_string())?;
    let starter_hash=starter.parent().and_then(Path::file_name).and_then(|s|s.to_str()).ok_or("launcher_bootstrap_hash_missing")?;
    if !installation.manifest.allowed_launcher_hashes.iter().any(|hash|hash==starter_hash) {
        return Err("launcher_bootstrap_not_allowed".into());
    }
    let mut command=Command::new(&runtime.executable);
    command.arg("mcp").env("SE_MCP_STARTER_SHA256",starter_hash);
    // Keep parent lifetime admission until the child acquired its own locks.
    let mut child=os::spawn_verified(command,image,true,false,false)?;
    drop(control);
    drop(lifetime);
    let status=child.wait().map_err(|e|format!("runtime_wait_failed: {e}"))?;
    Ok(status.code().unwrap_or(1))
}

pub fn spawn_generation_child(mut command:Command,input:bool)->Result<GenerationChild,String> {
    let Some(runtime)=crate::mcp_launcher::current_runtime() else {
        return os::spawn_legacy(command,input);
    };
    if command.get_program()!=runtime.executable.as_os_str() {
        return Err("generation_child_image_mismatch".into());
    }
    command.env_remove("SE_MCP_ADMISSION_CHANNEL")
        .env_remove("SE_MCP_LIFECYCLE").env_remove("SE_MCP_LIFECYCLE_HANDLE")
        .env_remove("SE_MCP_STARTER_SHA256").env_remove("SE_MCP_MIGRATION_ENTRY");
    let _parent_operation=crate::mcp_launcher::admission::continuation_operation()?;
    let _parent_lifetime=runtime.lifetime()?;
    let arguments=command.get_args().map(|a|a.to_string_lossy().into_owned()).collect::<Vec<_>>();
    if is_maintenance(&arguments) {
        let installation=Installation::load(&runtime.installation.root)?;
        let path=installation.installed_cli();
        let image=os::open_verified(&installation.root,&path,&installation.manifest.runtime_sha256)?;
        let mut current=Command::new(&path);
        current.args(command.get_args());
        for (key,value) in command.get_envs() {
            if let Some(value)=value {current.env(key,value);} else {current.env_remove(key);}
        }
        current.env("SE_MCP_MAINTENANCE","1");
        os::spawn_verified(current,image,false,true,input)
    } else {
        let image=os::open_verified(&runtime.installation.root,&runtime.executable,&runtime.contract.runtime_sha256)?;
        os::spawn_verified(command,image,false,true,input)
    }
}

pub struct LifecycleWatch {dead:Arc<AtomicBool>}
impl LifecycleWatch {
    pub fn arm()->Result<Self,String> {
        let dead=Arc::new(AtomicBool::new(false));
        if let Some(mut reader)=crate::mcp_launcher::admission::LIFECYCLE.lock().map_err(|_|"runtime_lifecycle_poisoned")?.take() {
            let ended=dead.clone();
            std::thread::Builder::new().name("se-mcp-lifecycle".into()).spawn(move || {
                let mut byte=[0u8];
                loop {
                    match reader.read(&mut byte) {
                        Err(e) if e.kind()==std::io::ErrorKind::Interrupted=>{},
                        _=>break,
                    }
                }
                ended.store(true,Ordering::Release);
            }).map_err(|e|e.to_string())?;
        }
        Ok(Self{dead})
    }
    pub fn parent_ended(&self)->bool {self.dead.load(Ordering::Acquire)}
}

/// The public legacy entry remains usable for its one-time profile migration.
pub fn launch_from_installed()->Result<Option<i32>,String> {
    if crate::mcp_launcher::current_runtime().is_some() {return Ok(None);}
    let installation=match Installation::current() {
        Ok(Some(installation))=>installation,
        Ok(None)=>return Ok(None),
        Err(error) if error.contains("runtime_installation_lock_filesystem_unsupported")
            || error.contains("runtime_network_installation_unsupported")=>{
                eprintln!("se mcp: managed runtime unavailable on this installation filesystem; using the legacy stop contract");
                return Ok(None);
            },
        Err(error)=>return Err(error),
    };
    let path=installation.launcher_path();
    let image=os::open_verified(&installation.root,&path,&installation.manifest.launcher_sha256)?;
    eprintln!("se mcp: one-time migration through installed se; this session follows the normal update-stop contract. Reconnect through the registered launcher after the profile refresh.");
    let mut command=Command::new(&path);command.env("SE_MCP_MIGRATION_ENTRY","1");
    os::run_stable_launcher(command,image).map(Some)
}

pub fn is_maintenance(arguments:&[String])->bool {
    arguments.first().is_some_and(|a|a=="update")
        || (arguments.first().is_some_and(|a|a=="plugin")
            && matches!(arguments.get(1).map(String::as_str),Some("install"|"uninstall"|"refresh")))
}

pub fn refresh_current_profile()->Result<(),String> {
    let Some(runtime)=crate::mcp_launcher::current_runtime() else {return Ok(());};
    let _admitted=crate::mcp_launcher::operation()?;
    let mut command=Command::new(&runtime.executable);
    command.args(["plugin","refresh"]);
    let mut child=spawn_generation_child(command,false)?;
    std::thread::Builder::new().name("se-profile-refresh".into()).spawn(move || {
        let output=child.stdout.take();
        let errors=child.stderr.take();
        let err_reader=errors.map(|mut pipe|std::thread::spawn(move || {
            let mut text=String::new();
            let _=pipe.by_ref().take(65536).read_to_string(&mut text);
            text
        }));
        if let Some(mut output)=output {let _=std::io::copy(&mut output,&mut std::io::sink());}
        let _=child.wait();
        if let Some(reader)=err_reader {
            if let Ok(text)=reader.join() {if !text.is_empty() {eprint!("{text}");}}
        }
    }).map_err(|e|e.to_string())?;
    Ok(())
}

pub fn start_current_daemon(runtime:&RuntimeIdentity)->Result<(),String> {
    let installation=Installation::load(&runtime.installation.root)?;
    let _operation=crate::mcp_launcher::operation()?;
    let path=installation.installed_cli();
    let image=os::open_verified(&installation.root,&path,&installation.manifest.runtime_sha256)?;
    let mut command=Command::new(&path);
    command.arg("--sync-daemon");
    os::spawn_current_daemon(command,image)
}
