use std::{collections::{BTreeMap,HashSet}, fs, io::{self,Read}, path::{Path,PathBuf}};
use std::os::unix::fs::{MetadataExt,PermissionsExt};
use sha2::{Digest,Sha256};
use crate::apps::core::model::{AppId,AppKind,CatalogApp};
use super::super::shared::platform::InstalledApp;
use super::desktop_entry::{self as entry, unescape};

const MAX_ENTRY_BYTES:u64=256*1024;
const MAX_CATALOG_FILES:usize=8192;
pub(super) enum Command {
    Exec {program:PathBuf,args:Vec<String>,directory:Option<PathBuf>, executable_stamp:String},
    Dbus {service:String,path:String},
    Unavailable,
}
pub(crate) struct Target {pub(super) file:PathBuf,pub(super) source_hash:String,pub(super) command:Command}
fn failed(reason:&str)->io::Error {io::Error::new(io::ErrorKind::InvalidData,reason)}
pub(super) fn read(path:&Path)->io::Result<String> {
    let file=fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len()>MAX_ENTRY_BYTES {return Err(failed("desktop entry exceeds byte budget"));}
    let mut bytes=Vec::new(); file.take(MAX_ENTRY_BYTES+1).read_to_end(&mut bytes)?;
    if bytes.len() as u64>MAX_ENTRY_BYTES {return Err(failed("desktop entry grew beyond byte budget"));}
    String::from_utf8(bytes).map_err(|_|failed("desktop entry is not UTF-8"))
}
pub(super) fn hash(text:&str)->String {format!("{:x}",Sha256::digest(text.as_bytes()))}
pub(super) fn stamp(path:&Path)->io::Result<String> {
    let m=fs::metadata(path)?;
    if !m.is_file() || m.permissions().mode()&0o111==0 {return Err(failed("application executable is unavailable"));}
    Ok(format!("{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",m.dev(),m.ino(),m.uid(),m.gid(),m.mode(),m.len(),m.mtime(),m.mtime_nsec(),m.ctime(),m.ctime_nsec()))
}
fn executable(name:&str)->Option<PathBuf> {
    let path=Path::new(name);
    let candidate=if path.is_absolute() {Some(path.to_owned())}
    else if name.contains('/') {None}
    else {std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|dir|dir.is_absolute()).map(|dir|dir.join(name)).find(|p|stamp(p).is_ok())};
    candidate.and_then(|p|fs::canonicalize(p).ok()).filter(|p|stamp(p).is_ok())
}
fn dirs()->io::Result<Vec<PathBuf>> {
    let user=std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p|p.is_absolute())
        .unwrap_or(super::session::home()?.join(".local/share"));
    let mut out=vec![user];
    let system=std::env::var_os("XDG_DATA_DIRS").filter(|s|!s.is_empty())
        .unwrap_or_else(||"/usr/local/share:/usr/share".into());
    out.extend(std::env::split_paths(&system).filter(|p|p.is_absolute()));
    Ok(out)
}
fn localized(values:&BTreeMap<String,String>)->io::Result<String> {
    let locale=std::env::var("LC_ALL").ok().filter(|s|!s.is_empty())
        .or_else(||std::env::var("LC_MESSAGES").ok().filter(|s|!s.is_empty()))
        .or_else(||std::env::var("LANG").ok()).unwrap_or_default();
    let (locale,modifier)=locale.split_once('@').map_or((locale.as_str(),None),|(base,m)|(base,Some(m)));
    let base=locale.split('.').next().unwrap_or_default();
    let language=base.split('_').next().unwrap_or(base);
    let mut keys=Vec::new();
    if let Some(modifier)=modifier {keys.push(format!("Name[{base}@{modifier}]"));}
    keys.push(format!("Name[{base}]"));
    if language!=base {
        if let Some(modifier)=modifier {keys.push(format!("Name[{language}@{modifier}]"));}
        keys.push(format!("Name[{language}]"));
    }
    keys.push("Name".into());
    keys.iter().find_map(|key|values.get(key)).map(|s|unescape(s)).transpose()?
        .filter(|s|!s.is_empty()).ok_or_else(||failed("desktop entry has no name"))
}
fn shown(v:&BTreeMap<String,String>)->bool {
    if v.get("Hidden").is_some_and(|s|s=="true") || v.get("NoDisplay").is_some_and(|s|s=="true") {return false;}
    let desktops:HashSet<String>=std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().split(':').map(str::to_owned).collect();
    if v.get("OnlyShowIn").is_some_and(|s|!s.split(';').any(|d|desktops.contains(d))) {return false;}
    !v.get("NotShowIn").is_some_and(|s|s.split(';').any(|d|desktops.contains(d)))
}
fn dbus_target(id:&str)->Option<Command> {
    let service=id.strip_suffix(".desktop")?;
    if !service.contains('.') || service.split('.').any(|part|part.is_empty()
        || part.as_bytes()[0].is_ascii_digit() || !part.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_'||b==b'-')) {return None;}
    Some(Command::Dbus {service:service.into(),path:format!("/{}",service.replace('.',"/").replace('-',"_"))})
}
fn command(v:&BTreeMap<String,String>,id:&str,name:&str,path:&Path)->io::Result<Command> {
    if let Some(check)=v.get("TryExec") {
        if executable(&unescape(check)?).is_none() {return Err(failed("TryExec executable is unavailable"));}
    }
    if v.get("DBusActivatable").is_some_and(|s|s=="true") {
        return dbus_target(id).ok_or_else(||failed("invalid D-Bus application identity"));
    }
    let exec=v.get("Exec").ok_or_else(||failed("desktop entry has no Exec"))?;
    let icon=v.get("Icon").map(|s|unescape(s)).transpose()?.unwrap_or_default();
    let filename=path.to_str().ok_or_else(||failed("desktop entry path is not UTF-8"))?;
    let mut args=entry::command(exec,name,&icon,filename)?;
    let mut program=executable(&args.remove(0)).ok_or_else(||failed("Exec executable is unavailable"))?;
    if v.get("Terminal").is_some_and(|s|s=="true") {
        let terminal=executable("xdg-terminal-exec").ok_or_else(||failed("Terminal=true requires installed xdg-terminal-exec"))?;
        args.insert(0,program.to_str().ok_or_else(||failed("executable path is not UTF-8"))?.into());
        args.insert(0,"--".into()); program=terminal;
    }
    let directory=v.get("Path").map(|s|unescape(s)).transpose()?.filter(|s|!s.is_empty()).map(PathBuf::from);
    if directory.as_ref().is_some_and(|p|!p.is_absolute() || !p.is_dir()) {return Err(failed("desktop-entry working directory is unavailable"));}
    let executable_stamp=stamp(&program)?;
    Ok(Command::Exec {program,args,directory,executable_stamp})
}
pub(super) fn list()->io::Result<Vec<InstalledApp<Target>>> {
    let mut found=BTreeMap::<String,Option<InstalledApp<Target>>>::new();
    let mut scanned=0usize;
    for base in dirs()? {
        let root=base.join("applications");
        let mut pending=vec![(root.clone(),0usize)];
        let mut local=HashSet::new();
        while let Some((dir,depth))=pending.pop() {
            let children=match fs::read_dir(&dir) {Ok(v)=>v,Err(e) if e.kind()==io::ErrorKind::NotFound=>continue,Err(e)=>return Err(e)};
            for child in children {
                let child=child?; scanned+=1;
                if scanned>MAX_CATALOG_FILES {return Err(failed("application catalog exceeds complete-scan budget"));}
                let path=child.path(); let kind=child.file_type()?;
                if kind.is_dir() {
                    if depth>=8 {return Err(failed("application catalog exceeds directory-depth budget"));}
                    pending.push((path,depth+1)); continue;
                }
                if path.extension().and_then(|s|s.to_str())!=Some("desktop") {continue;}
                let id=path.strip_prefix(&root).ok().and_then(|p|p.to_str()).map(|s|s.replace('/',"-"))
                    .ok_or_else(||failed("desktop identity is not UTF-8"))?;
                if !local.insert(id.clone()) {return Err(failed("ambiguous desktop-file ID"));}
                if found.contains_key(&id) {continue;}
                // Record even hidden/invalid user overrides, never resurrect a
                // lower-priority system application with the same identity.
                found.insert(id.clone(),None);
                let Ok(text)=read(&path) else {continue;};
                let Ok(v)=entry::values(&text) else {continue;};
                if v.get("Type").map(String::as_str)!=Some("Application") || !shown(&v) {continue;}
                let Ok(name)=localized(&v) else {continue;};
                let (command,reason)=match command(&v,&id,&name,&path) {Ok(c)=>(c,None),Err(e)=>(Command::Unavailable,Some(e.to_string()))};
                let source_hash=hash(&text);
                let target=Target {file:path.clone(),source_hash:source_hash.clone(),command};
                let fingerprint=match &target.command {
                    Command::Exec {program,args,directory,executable_stamp}=>format!("{source_hash}:{path:?}:{program:?}:{args:?}:{directory:?}:{executable_stamp}"),
                    Command::Dbus {service,path:object}=>format!("{source_hash}:{path:?}:dbus:{service}:{object}"),
                    Command::Unavailable=>format!("{source_hash}:{path:?}:unavailable"),
                };
                let app=CatalogApp {app_id:AppId::try_from(id.clone())?,name,kind:AppKind::LinuxDesktopEntry,
                    icon:None,can_launch:reason.is_none(),unavailable_reason:reason};
                found.insert(id,Some(InstalledApp {app,fingerprint,target}));
            }
        }
    }
    Ok(found.into_values().flatten().collect())
}
