# Terminal discoverability and `se update` — recorded API syntax

Checked 2026-09-26 against the vendor documentation of the versions pinned in
`native/Cargo.lock` (clap 4.6.1, ctrlc 3.5.2, libc 0.2.186, windows-sys 0.59.0,
chrono 0.4.44, crossbeam-channel 0.5.15) and the Rust standard library.

## clap 4 (derive)

Source: docs.rs clap, `_derive/_cookbook/git_derive` (stash example) and
`builder::Command::args_conflicts_with_subcommands`.

- Parent arguments that apply when no subcommand is given:

  ```rust
  #[derive(Args)]
  #[command(args_conflicts_with_subcommands = true)]
  #[command(flatten_help = true)]
  struct StashArgs {
      #[command(subcommand)]
      command: Option<StashCommands>,
      #[command(flatten)]
      push: StashPushArgs,
  }
  ```

  "Specifies that use of an argument prevents the use of subcommands"; the
  arguments then only follow the final subcommand.
- Ranged integer: `#[arg(value_parser = clap::value_parser!(u64).range(1..))]`
  ("0 is not in 1..=…" is reported by clap).
- `Arg::hide(true)` hides an argument from help; `conflicts_with = "name"`.

## ctrlc 3.5.2

Source: docs.rs `ctrlc::set_handler`.

- `pub fn set_handler<F>(user_handler: F) -> Result<(), Error> where F: FnMut() + 'static + Send`
- Unix: SIGINT (SIGTERM/SIGHUP only with the `termination` feature); Windows:
  console control handler. "Starts a new dedicated signal handling thread".
- Errors on a system failure; a second registration fails (`MultipleHandlers`).
  Existing Unix handlers for the same signal are overwritten.

## POSIX terminal echo (libc 0.2.186, Linux target)

Sources: man7.org `termios(3)`, docs.rs `libc::tcsetattr`, `libc::termios`,
`libc::ECHONL`.

- `pub unsafe extern "C" fn tcgetattr(fd: c_int, termios: *mut termios) -> c_int`
- `pub unsafe extern "C" fn tcsetattr(fd: c_int, optional_actions: c_int, termios: *const termios) -> c_int`
- 0 on success, -1 with errno on failure; "tcsetattr() returns success if any
  of the requested changes could be successfully carried out".
- `termios` is `Copy + Clone + Send`; `c_lflag: tcflag_t`.
- `ECHO` echoes input; `ECHONL` echoes the NL even without `ECHO`
  (`pub const ECHONL: tcflag_t = 0x40`); `TCSANOW` applies immediately.

## Windows console echo (windows-sys 0.59.0, feature `Win32_System_Console`)

Sources: learn.microsoft.com `SetConsoleMode`, `GetConsoleMode`,
`GetStdHandle`; docs.rs windows-sys 0.59.0 item pages and feature list
(`Win32_System_Console` depends only on `Win32_System`).

- `pub unsafe extern "system" fn GetStdHandle(nstdhandle: STD_HANDLE) -> HANDLE`
  — `INVALID_HANDLE_VALUE` on failure, NULL without standard handles.
- `pub unsafe extern "system" fn GetConsoleMode(hconsolehandle: HANDLE, lpmode: *mut CONSOLE_MODE) -> BOOL`
- `pub unsafe extern "system" fn SetConsoleMode(hconsolehandle: HANDLE, dwmode: CONSOLE_MODE) -> BOOL`
  — nonzero on success.
- `pub const STD_INPUT_HANDLE: STD_HANDLE = 4294967286u32;`
  `pub const ENABLE_ECHO_INPUT: CONSOLE_MODE = 4u32;`
  `pub type HANDLE = *mut c_void;` `INVALID_HANDLE_VALUE` = all-ones pointer
  (`Win32::Foundation`).
- `ENABLE_ECHO_INPUT`: characters read by `ReadFile`/`ReadConsole` are echoed;
  usable only with `ENABLE_LINE_INPUT`. `ENABLE_PROCESSED_INPUT`: Ctrl+C is
  processed by the system (handler), not placed in the buffer.

## Rust standard library

- `std::env::current_exe`: "If the executable was invoked through a symbolic
  link, some platforms will return the path of the symbolic link and other
  platforms will return the path of the symbolic link's target." → resolve
  explicitly with `std::fs::canonicalize` on Linux.
- `std::fs::canonicalize`: Unix `realpath`; Windows returns `\\?\` extended
  paths "and it may be incompatible with other applications (if passed to the
  application on the command-line…)" → not used for Windows helper arguments.
- `std::fs::rename`: Unix `rename(2)`; POSIX: "a directory entry named new
  shall remain visible to other threads throughout the renaming operation and
  refer either to the file referred to by new or old"; different file systems
  fail (`EXDEV`) → stage the pending file beside the target.
- `std::io::IsTerminal` (stable 1.70): implemented for `Stdin`; `false` on
  unknown platforms or errors; Windows also treats msys/cygwin ptys as
  terminals (console APIs then fail — reported, not echoed).
- `std::fs::set_permissions` + `Permissions` (portable);
  `std::os::unix::fs::PermissionsExt::from_mode(0o755)` (Unix only).

## chrono 0.4.44 (`clock` → `now` → `std` → `alloc`)

- `DateTime::<Utc>::from_timestamp(secs: i64, nsecs: u32) -> Option<Self>`
- `fn with_timezone<Tz2: TimeZone>(&self, tz: &Tz2) -> DateTime<Tz2>`
- `format(&str)` needs `alloc`; `%Y-%m-%d %H:%M:%S %:z` → `2026-09-26 14:05:00 +02:00`.

## crossbeam-channel 0.5.15

- "Channels are concurrent FIFO queues used for passing messages between
  threads" (crate docs). The Share worker sends a discovery event before it
  acknowledges the command on a second channel, so the event is already
  queued when the daemon receives the acknowledgement.
