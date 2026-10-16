//! Where the Linux exec host looks bare program names up for a grant with a
//! program list: the fixed system path of `exec_supervisor_protocol.rs`
//! (OpenSSH's `_PATH_STDPATH` with the local prefixes), never the `PATH` of
//! the caller or of the worker.
use std::path::PathBuf;

use crate::share::exec_policy::program_path::ProgramSearch;
use crate::share::exec_supervisor_protocol::ACCOUNT_PATH;

pub(super) fn fixed() -> ProgramSearch {
    ProgramSearch {
        directories: ACCOUNT_PATH.split(':').map(PathBuf::from).collect(),
        suffixes: &[],
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn luecken_task_the_fixed_search_path_is_absolute_system_directories() {
        let search = super::fixed();
        assert!(search.directories.len() >= 4);
        assert!(search
            .directories
            .iter()
            .all(|directory| directory.is_absolute()));
        assert!(search.directories.contains(&"/usr/bin".into()));
        assert!(search.suffixes.is_empty());
    }
}
