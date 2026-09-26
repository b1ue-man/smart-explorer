use std::ffi::c_void;
use std::fs::File;
use std::io::{self, Write};
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetStdHandle, SetConsoleMode, CONSOLE_MODE, ENABLE_ECHO_INPUT, STD_INPUT_HANDLE,
};

pub(super) fn local_path(path: &str) -> std::path::PathBuf {
    let rooted;
    let bytes = path.as_bytes();
    let path = if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        rooted = format!("{path}/");
        rooted.as_str()
    } else {
        path
    };
    std::path::PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
#[derive(Default)]
struct FileInformation {
    attributes: u32,
    creation_time: FileTime,
    last_access_time: FileTime,
    last_write_time: FileTime,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[link(name = "kernel32")]
extern "system" {
    #[link_name = "GetFileInformationByHandle"]
    fn get_file_information_by_handle(
        handle: *mut c_void,
        information: *mut FileInformation,
    ) -> i32;
}

pub(super) fn same_file(left: &str, right: &str) -> io::Result<bool> {
    // Keep both handles open while comparing their indexes. Windows may reuse
    // an index after the corresponding handle closes.
    let left = File::open(left)?;
    let right = File::open(right)?;
    Ok(file_key(&left)? == file_key(&right)?)
}

fn file_key(file: &File) -> io::Result<(u32, u64)> {
    let mut information = FileInformation::default();
    // SAFETY: `file` owns a valid handle for this call, and `information` is a
    // writable, correctly laid-out output structure that lives until it returns.
    let ok =
        unsafe { get_file_information_by_handle(file.as_raw_handle(), &mut information as *mut _) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // The legacy index can collide on ReFS; the only consequence here is a
    // conservative refusal to copy, never a missed self-target check.
    let index =
        (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low);
    Ok((information.volume_serial_number, index))
}

pub(super) fn validate_connection_protocol(
    _protocol: crate::creds::Protocol,
) -> Result<(), String> {
    Ok(())
}

pub(super) fn read_hidden_line(prompt: &str) -> Result<String, String> {
    // SAFETY: GetStdHandle only reads this process's standard handle table.
    let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    if input.is_null() || input == INVALID_HANDLE_VALUE {
        return Err("no console input is attached".to_string());
    }
    let mut saved: CONSOLE_MODE = 0;
    // SAFETY: `input` is the standard input handle and `saved` a writable mode.
    if unsafe { GetConsoleMode(input, &mut saved) } == 0 {
        return Err(format!("console mode: {}", io::Error::last_os_error()));
    }
    ctrlc::set_handler(move || {
        // SAFETY: restores the mode read above on the standard input console.
        unsafe { SetConsoleMode(GetStdHandle(STD_INPUT_HANDLE), saved) };
        std::process::exit(130);
    })
    .map_err(|error| format!("Ctrl+C handler: {error}"))?;
    let mut stderr = io::stderr();
    write!(stderr, "{prompt}")
        .and_then(|()| stderr.flush())
        .map_err(|error| error.to_string())?;
    // SAFETY: same console handle; only the echo flag is cleared.
    if unsafe { SetConsoleMode(input, saved & !ENABLE_ECHO_INPUT) } == 0 {
        return Err(format!("hide console input: {}", io::Error::last_os_error()));
    }
    let mut line = String::new();
    let read = io::stdin().read_line(&mut line);
    // SAFETY: restores the mode read above on the same handle.
    let restored = unsafe { SetConsoleMode(input, saved) };
    // Without echo the console does not move past the entered line either.
    let _ = writeln!(stderr);
    read.map_err(|error| format!("read hidden input: {error}"))?;
    if restored == 0 {
        return Err(format!("restore console input: {}", io::Error::last_os_error()));
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}
