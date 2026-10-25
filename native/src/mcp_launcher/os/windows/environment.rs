//! Windows environment names are merged and sorted using OS ordinal casing.
use std::{cmp::Ordering,ffi::{OsStr,OsString},os::windows::ffi::OsStrExt,process::Command};
#[link(name="kernel32")]
extern "system" {
    #[link_name="CompareStringOrdinal"]
    fn compare_string_ordinal(left:*const u16,left_len:i32,right:*const u16,right_len:i32,ignore_case:i32)->i32;
}
fn compare(left:&OsStr,right:&OsStr)->Result<Ordering,String> {
    let left=left.encode_wide().collect::<Vec<_>>();
    let right=right.encode_wide().collect::<Vec<_>>();
    let left_len=i32::try_from(left.len()).map_err(|_|"runtime_environment_name_too_long")?;
    let right_len=i32::try_from(right.len()).map_err(|_|"runtime_environment_name_too_long")?;
    // SAFETY: both complete UTF-16 buffers remain live; lengths are checked,
    // and TRUE is exactly 1 as CompareStringOrdinal requires.
    match unsafe {compare_string_ordinal(left.as_ptr(),left_len,right.as_ptr(),right_len,1)} {
        1=>Ok(Ordering::Less),2=>Ok(Ordering::Equal),3=>Ok(Ordering::Greater),
        _=>Err("runtime_environment_name_comparison_failed".into()),
    }
}
pub(super) fn block(command:&Command)->Result<Vec<u16>,String> {
    let mut env=std::env::vars_os().collect::<Vec<(OsString,OsString)>>();
    for (key,value) in command.get_envs() {
        let mut retained=Vec::with_capacity(env.len()+1);
        for (existing,text) in env {
            if compare(&existing,key)?!=Ordering::Equal {retained.push((existing,text));}
        }
        if let Some(value)=value {retained.push((key.into(),value.into()));}
        env=retained;
    }
    let mut failure=None;
    env.sort_by(|(left,_),(right,_)|match compare(left,right) {
        Ok(order)=>order,
        Err(error)=>{failure=Some(error);Ordering::Equal},
    });
    if let Some(error)=failure {return Err(error);}
    let mut block=Vec::new();
    for (key,value) in env {
        block.extend(key.encode_wide());block.push(b'=' as u16);
        block.extend(value.encode_wide());block.push(0);
    }
    if block.is_empty() {block.push(0);}
    block.push(0);Ok(block)
}
