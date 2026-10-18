//! A recorded conflict keeps the job's version location and retention. Loading
//! this one configuration never depends on another job being healthy. A
//! vault job (C5.8) is read from `vault-jobs/` and answers with its vault
//! options, so its conflicts resolve under the same settings as its runs.
use std::io;

pub(crate) fn recorded_options(id: &str) -> io::Result<crate::bisync::BisyncOptions> {
    if id.is_empty() || id != super::persistence::san_id(id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe sync job id",
        ));
    }
    let general = super::persistence::load_job_file(&super::persistence::job_file(
        &super::persistence::jobs_dir(),
        id,
    ));
    let job = match general {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match super::vault_jobs_dir::find_in(&super::vault_jobs_dir::vault_jobs_dir(), id)? {
                Some(vault) => {
                    return vault
                        .checked_vault_opts(false)
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
                }
                None => return Err(error),
            }
        }
        other => other?,
    };
    job.checked_opts(false)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}
