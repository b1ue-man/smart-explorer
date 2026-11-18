//! The lifetime of a host process where neither a job object (Windows)
//! nor a parent-death signal (Linux) binds it: it ends with the end of its
//! input alone (moved out of `os/mod.rs`, which stays below 500 lines).

pub(super) fn prepare(_command: &mut std::process::Command) {}

pub(super) fn bind(_child: &std::process::Child) -> std::io::Result<()> {
    Ok(())
}

pub(super) fn arm_worker() -> std::io::Result<()> {
    Ok(())
}
