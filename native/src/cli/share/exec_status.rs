use clap::{Args, Subcommand};
use clap_complete::ArgValueCandidates;

#[derive(Args)]
pub(super) struct ExecStatusArgs {
    #[arg(long, global = true, help = "Print machine-readable JSON")]
    json: bool,
    #[command(subcommand)]
    command: Option<ExecStatusCommand>,
}

#[derive(Subcommand)]
enum ExecStatusCommand {
    #[command(about = "List active incoming and outgoing executions")]
    List,
    #[command(about = "Show one active or recent execution")]
    Show(Selector),
    #[command(about = "Cancel one active execution; omit selector when exactly one is active")]
    Cancel(OptionalSelector),
    #[command(about = "List bounded redacted recent execution history")]
    History,
}

#[derive(Args)]
struct Selector {
    #[arg(add = ArgValueCandidates::new(crate::cli::completions::exec_id_candidates))]
    selector: String,
}

#[derive(Args)]
struct OptionalSelector {
    #[arg(add = ArgValueCandidates::new(crate::cli::completions::exec_id_candidates))]
    selector: Option<String>,
}

pub(super) fn run(args: ExecStatusArgs) -> Result<(), String> {
    let snapshot = crate::daemon::exec_jobs()?;
    match args.command {
        None | Some(ExecStatusCommand::List) => {
            print(active(&snapshot), args.json)?;
            if !args.json {
                println!("history\tse share exec history");
            }
            Ok(())
        }
        Some(ExecStatusCommand::History) => print(history(&snapshot), args.json),
        Some(ExecStatusCommand::Show(selector)) => {
            let all = active(&snapshot)
                .into_iter()
                .chain(history(&snapshot))
                .collect::<Vec<_>>();
            let selected = select(&all, Some(&selector.selector), "execution")?;
            print(vec![selected], args.json)
        }
        Some(ExecStatusCommand::Cancel(selector)) => {
            let jobs = active(&snapshot);
            let selected = select(&jobs, selector.selector.as_deref(), "active execution")?;
            let id = selected.view.exec_id.clone();
            if !crate::daemon::cancel_exec(cancel_target(selected))? {
                return Err(format!("execution is no longer active: {id}"));
            }
            if args.json {
                println!(
                    "{}",
                    serde_json::json!({"exec_id": id, "cancel_requested": true})
                );
            } else {
                println!("exec\t{id}\tstate=cancelling");
            }
            Ok(())
        }
    }
}

fn cancel_target(job: JobChoice<'_>) -> crate::daemon::ExecCancelTarget {
    crate::daemon::ExecCancelTarget {
        direction: job.direction,
        exec_id: job.view.exec_id.clone(),
        peer_device_id: job.view.peer_device_id.clone(),
    }
}

#[derive(Clone, Copy, Debug)]
struct JobChoice<'a> {
    direction: crate::daemon::ExecJobDirection,
    view: &'a crate::share::ExecJobView,
}

fn active(snapshot: &crate::daemon::ExecJobsSnapshot) -> Vec<JobChoice<'_>> {
    snapshot
        .outgoing_active
        .iter()
        .map(|view| JobChoice {
            direction: crate::daemon::ExecJobDirection::Outgoing,
            view,
        })
        .chain(snapshot.incoming_active.iter().map(|view| JobChoice {
            direction: crate::daemon::ExecJobDirection::Incoming,
            view,
        }))
        .collect()
}

fn history(snapshot: &crate::daemon::ExecJobsSnapshot) -> Vec<JobChoice<'_>> {
    snapshot
        .outgoing_history
        .iter()
        .map(|view| JobChoice {
            direction: crate::daemon::ExecJobDirection::Outgoing,
            view,
        })
        .chain(snapshot.incoming_history.iter().map(|view| JobChoice {
            direction: crate::daemon::ExecJobDirection::Incoming,
            view,
        }))
        .collect()
}

fn select<'a>(
    jobs: &'a [JobChoice<'a>],
    selector: Option<&str>,
    kind: &str,
) -> Result<JobChoice<'a>, String> {
    let matches = jobs
        .iter()
        .copied()
        .filter(|job| {
            selector.is_none_or(|selector| {
                selector.eq_ignore_ascii_case(&crate::cli::completions::exec_job_selector(
                    job.direction,
                    &job.view.exec_id,
                )) || exact_or_prefix(selector, job.view.exec_id.as_str())
                    || exact_or_prefix(selector, &job.view.peer_device_id)
                    || job.view.peer_device_name.eq_ignore_ascii_case(selector)
            })
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [job] => Ok(*job),
        [] => Err(format!(
            "{kind} not found; run `se share exec` to list valid selectors"
        )),
        _ => Err(format!(
            "multiple {kind}s match; choose one exact selector: {}",
            matches
                .iter()
                .map(|job| {
                    crate::cli::completions::exec_job_selector(job.direction, &job.view.exec_id)
                })
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn print(jobs: Vec<JobChoice<'_>>, json: bool) -> Result<(), String> {
    if json {
        let values = jobs
            .iter()
            .map(|job| {
                serde_json::json!({
                    "direction": match job.direction {
                        crate::daemon::ExecJobDirection::Outgoing => "outgoing",
                        crate::daemon::ExecJobDirection::Incoming => "incoming",
                    },
                    "job": job.view,
                })
            })
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&values).map_err(|error| error.to_string())?
        );
    } else if jobs.is_empty() {
        println!("exec\t0");
    } else {
        for job in jobs {
            let view = job.view;
            println!(
                "exec\t{}\tdirection={:?}\tpeer={}\tprogram={}\tstate={:?}\tpolicy_revision={}\tstarted_at={}\tfinished_at={}",
                view.exec_id, job.direction,
                clean(&view.peer_device_name), clean(&view.program), view.state,
                view.policy_revision,
                view.started_at.map_or_else(|| "-".into(), |value| value.to_string()),
                view.finished_at.map_or_else(|| "-".into(), |value| value.to_string()),
            );
        }
    }
    Ok(())
}

fn exact_or_prefix(selector: &str, candidate: &str) -> bool {
    candidate.eq_ignore_ascii_case(selector)
        || (selector.len() >= 4
            && candidate
                .to_ascii_lowercase()
                .starts_with(&selector.to_ascii_lowercase()))
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colliding_exec_ids_are_ambiguous_until_peer_selects_exact_direction() {
        let id = crate::share::ExecId::parse("11".repeat(16)).unwrap();
        let snapshot = crate::daemon::ExecJobsSnapshot {
            outgoing_active: vec![job(id.clone(), "out-peer", "Outgoing")],
            incoming_active: vec![job(id.clone(), "in-peer", "Incoming")],
            ..Default::default()
        };
        let jobs = active(&snapshot);
        let error = select(&jobs, Some(id.as_str()), "active execution").unwrap_err();
        assert!(error.contains("multiple active executions match"));
        assert!(error.contains(&format!("outgoing:{id}")));
        assert!(error.contains(&format!("incoming:{id}")));

        let incoming = select(&jobs, Some(&format!("incoming:{id}")), "active execution").unwrap();
        let target = cancel_target(incoming);
        assert_eq!(target.direction, crate::daemon::ExecJobDirection::Incoming);
        assert_eq!(target.exec_id, id);
        assert_eq!(target.peer_device_id, "in-peer");
    }

    fn job(
        exec_id: crate::share::ExecId,
        peer_device_id: &str,
        peer_device_name: &str,
    ) -> crate::share::ExecJobView {
        crate::share::ExecJobView {
            exec_id,
            peer_device_id: peer_device_id.into(),
            peer_device_name: peer_device_name.into(),
            program: "<shell>".into(),
            command_digest: "digest".into(),
            state: crate::share::ExecLifecycleState::Running,
            policy_revision: 1,
            started_at: Some(1),
            finished_at: None,
            terminal: None,
        }
    }
}
