use crate::mcp_launcher::{os,core::capability::{RuntimeProbe,LauncherProbe}};
use std::{path::Path,process::Command,ffi::OsString,io::Read};
/// Must run before admission: this harmless mode also probes an inactive new
/// generation whose installation has not committed a current manifest yet.
pub fn runtime_contract_probe(arguments:&[OsString])->Option<Result<i32,String>> {
    if arguments.first().is_none_or(|a|a!="--mcp-runtime-contract") {return None;}
    Some((|| {
        if arguments.len()!=1 {return Err("runtime contract probe accepts no arguments".into());}
        let probe=RuntimeProbe::current();probe.validate(env!("CARGO_PKG_VERSION"))?;
        if probe.compatibility_catalog.runtimes.iter().any(|r|!crate::mcp_launcher::core::manifest::implements_contract_pair(&r.ipc_contract_id,&r.shared_state_contract_id)) {return Err("release_catalog_contract_adapter_missing".into());}
        println!("{}",serde_json::to_string(&probe).map_err(|e|e.to_string())?);Ok(0)
    })())
}
pub(crate) fn verify_runtime(root:&Path,path:&Path,hash:&str,version:&str)->Result<RuntimeProbe,String> {
    let bytes=capture(root,path,hash,"--mcp-runtime-contract")?;
    let probe=serde_json::from_slice::<RuntimeProbe>(&bytes).map_err(|_|"runtime_capability_probe_unsupported")?;
    probe.validate(version)?;Ok(probe)
}
pub(crate) fn verify_launcher(root:&Path,path:&Path,hash:&str,version:&str)->Result<(),String> {
    let bytes=capture(root,path,hash,"--launcher-contract")?;
    serde_json::from_slice::<LauncherProbe>(&bytes).map_err(|_|"launcher_capability_probe_unsupported")?.validate(version)
}
fn capture(root:&Path,path:&Path,hash:&str,argument:&str)->Result<Vec<u8>,String> {
    let image=os::open_verified(root,path,hash)?;
    let mut command=Command::new(path);command.arg(argument).env_remove("COMPLETE");
    let mut child=os::spawn_installation_probe(command,image)?;
    let stdout=child.stdout.take();let stderr=child.stderr.take();
    let output=std::thread::Builder::new().name("se-contract-probe".into()).spawn(move || {
        let mut bytes=Vec::new();if let Some(pipe)=stdout {pipe.take(256*1024+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;}Ok::<_,String>(bytes)
    }).map_err(|e| {let _=child.kill();let _=child.wait();e.to_string()})?;
    let errors=std::thread::Builder::new().name("se-contract-probe-errors".into()).spawn(move || {
        let mut bytes=Vec::new();if let Some(pipe)=stderr {let _=pipe.take(16*1024).read_to_end(&mut bytes);}bytes
    }).map_err(|e| {let _=child.kill();let _=child.wait();e.to_string()})?;
    let deadline=std::time::Instant::now()+std::time::Duration::from_secs(15);
    let status=loop {
        match child.try_wait() {Ok(Some(status))=>break status,Ok(None)=>{},Err(error)=>{let _=child.kill();let _=child.wait();return Err(error.to_string());}}
        if std::time::Instant::now()>=deadline {let _=child.kill();let _=child.wait();return Err("installer_contract_probe_timeout".into());}
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let bytes=output.join().map_err(|_|"installer_contract_reader_failed")??;let _=errors.join();
    if bytes.len()>256*1024 {return Err("installer_contract_probe_exceeds_record_budget".into());}
    if !status.success() {return Err(if status.code()==Some(2) {"installer_contract_probe_exit_2"}else{"installer_contract_probe_failed"}.into());}
    Ok(bytes)
}

pub fn verify_legacy_product(root:&Path,path:&Path,hash:&str,version:&str)->Result<(),String> {
    let bytes=capture(root,path,hash,"--version")?;
    let expected=format!("se {version}");
    if std::str::from_utf8(&bytes).ok().map(str::trim)!=Some(expected.as_str()) {return Err("legacy_cli_product_version_mismatch".into());}
    Ok(())
}

pub(crate) fn verify_manifest_policy(root:&Path,target:&crate::mcp_launcher::Manifest)->Result<(),String> {
    let probe=verify_runtime(root,&root.join(&target.runtime_file),&target.runtime_sha256,&target.product_version)?;
    if target.ipc_contract_id!=probe.ipc_contract_id || target.shared_state_contract_id!=probe.shared_state_contract_id
        || target.compatibility_catalog_sha256!=probe.compatibility_catalog_sha256
        || target.supported_runtimes.iter().any(|runtime|!probe.compatibility_catalog.allows_runtime(runtime))
        || target.allowed_launcher_hashes.iter().any(|hash|hash!=&target.launcher_sha256 && !probe.compatibility_catalog.allows_launcher(hash)) {
        return Err("target_manifest_differs_from_verified_incoming_release_policy".into());
    }
    Ok(())
}

pub(crate) fn classify_runtime(root:&Path,path:&Path,hash:&str,version:&str)->Result<bool,String> {
    match verify_runtime(root,path,hash,version) {
        Ok(_)=>Ok(true),
        Err(error) if error=="installer_contract_probe_exit_2" && historical_legacy_version(version)=> {
            verify_legacy_product(root,path,hash,version)?;Ok(false)
        }
        Err(error)=>Err(format!("runtime_capability_not_proven for {version}: {error}")),
    }
}
fn historical_legacy_version(version:&str)->bool {
    let mut parts=version.split('.');
    let parsed=(parts.next().and_then(|v|v.parse::<u32>().ok()),parts.next().and_then(|v|v.parse::<u32>().ok()),parts.next().and_then(|v|v.parse::<u32>().ok()));
    matches!(parsed,(Some(major),Some(minor),Some(patch)) if (major,minor,patch)<=(0,5,184)) && parts.next().is_none()
}
