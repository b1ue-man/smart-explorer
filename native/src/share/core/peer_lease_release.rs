use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::backend::PeerBackend;
use super::framing::{decode_resp, recv_resp_wire, send_ctrl};
use super::io_deadline;
use super::wire::{Ctrl, FsRequest, FsResponse};

const BEST_EFFORT_RELEASE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RELEASE_WORKERS: usize = 4;
static ACTIVE_RELEASE_WORKERS: AtomicUsize = AtomicUsize::new(0);

impl PeerBackend {
    pub(super) fn release_current_mount_lease(&self) {
        let Ok(Some(token)) = self.mount_lease.take_releasable() else {
            return;
        };
        schedule_release(
            self.endpoint_source.clone(),
            self.identity.clone(),
            self.node.clone(),
            token,
        );
    }
}

impl Drop for PeerBackend {
    fn drop(&mut self) {
        self.release_current_mount_lease();
    }
}

fn schedule_release(
    source: super::peer_endpoint_source::PeerEndpointSource,
    identity: super::identity::ShareIdentity,
    node: std::sync::Arc<super::node::ShareIrohNode>,
    token: String,
) {
    if !try_reserve(&ACTIVE_RELEASE_WORKERS, MAX_RELEASE_WORKERS) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("share-lease-release".into())
        .stack_size(512 * 1024)
        .spawn(move || {
            let _slot = ReleaseSlot {
                active: &ACTIVE_RELEASE_WORKERS,
            };
            let deadline = Instant::now() + BEST_EFFORT_RELEASE_TIMEOUT;
            let Ok(endpoint) = source.current() else {
                return;
            };
            let Ok(mut opened) = node.open_stream_until(&endpoint, &identity, deadline) else {
                return;
            };
            let timeout = match io_deadline::remaining(deadline, "peer mount lease release") {
                Ok(timeout) => timeout,
                Err(_) => {
                    io_deadline::abort(&mut opened.send, &mut opened.recv);
                    let _ =
                        node.invalidate_outgoing_session(&opened.session_key, opened.generation);
                    return;
                }
            };
            let result = node.block_on(io_deadline::run_for(
                "peer mount lease release",
                timeout,
                async {
                    send_ctrl(
                        &mut opened.send,
                        &Ctrl::Fs {
                            req: FsRequest::ReleaseLease,
                            lease: Some(token),
                        },
                    )
                    .await?;
                    recv_resp_wire(&mut opened.recv).await
                },
            ));
            match result {
                Ok(response) => match decode_resp(response) {
                    Ok(FsResponse::Ok) => {}
                    Ok(_) => {
                        io_deadline::abort(&mut opened.send, &mut opened.recv);
                        let _ = node
                            .invalidate_outgoing_session(&opened.session_key, opened.generation);
                    }
                    Err(_) => io_deadline::abort(&mut opened.send, &mut opened.recv),
                },
                Err(_) => {
                    io_deadline::abort(&mut opened.send, &mut opened.recv);
                    let _ =
                        node.invalidate_outgoing_session(&opened.session_key, opened.generation);
                }
            }
        });
    if spawned.is_err() {
        ACTIVE_RELEASE_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn try_reserve(active: &AtomicUsize, limit: usize) -> bool {
    active
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            (current < limit).then_some(current + 1)
        })
        .is_ok()
}

struct ReleaseSlot<'a> {
    active: &'a AtomicUsize,
}

impl Drop for ReleaseSlot<'_> {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_drive_task_lease_release_timeout_is_shorter_than_normal_peer_io() {
        assert!(BEST_EFFORT_RELEASE_TIMEOUT < io_deadline::PEER_OP_TIMEOUT);
    }

    #[test]
    fn remote_drive_task_lease_release_workers_have_a_hard_bound() {
        let active = AtomicUsize::new(0);
        for _ in 0..MAX_RELEASE_WORKERS {
            assert!(try_reserve(&active, MAX_RELEASE_WORKERS));
        }
        assert!(!try_reserve(&active, MAX_RELEASE_WORKERS));
        {
            let _slot = ReleaseSlot { active: &active };
        }
        assert!(try_reserve(&active, MAX_RELEASE_WORKERS));
    }
}
