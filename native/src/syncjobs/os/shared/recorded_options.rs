//! A recorded conflict keeps the job's version location and retention. Loading
//! this one configuration never depends on another job being healthy.
use std::io;

pub(crate) fn recorded_options(id: &str) -> io::Result<crate::bisync::BisyncOptions> {
    if id.is_empty() || id != super::persistence::san_id(id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe sync job id",
        ));
    }
    let job = super::persistence::load_job_file(&super::persistence::job_file(
        &super::persistence::jobs_dir(),
        id,
    ))?;
    job.checked_opts(false)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}
