//! Remote program runs (`Ctrl::Exec`) over a Share connection: one stream
//! per run, bounded by the run's own timeout.
use std::io;
use std::time::{Duration, Instant};

use super::PeerBackend;
use crate::share::core::eio;
use crate::share::framing::{recv_ctrl, send_ctrl};
use crate::share::io_deadline;
use crate::share::types::{ExecRequest, ExecResult};
use crate::share::wire::Ctrl;

impl PeerBackend {
    pub fn exec(&self, req: ExecRequest) -> io::Result<ExecResult> {
        let started = Instant::now();
        let timeout = Duration::from_millis(req.timeout_ms.min(15 * 60 * 1_000))
            .saturating_add(Duration::from_secs(30))
            .min(Duration::from_secs(15 * 60 + 30));
        let endpoint = self.current_endpoint()?;
        let mut opened = self.node.open_stream_until(
            &endpoint,
            &self.identity,
            Instant::now() + crate::share::peer_request::CONTROL_ATTEMPT_TIMEOUT,
        )?;
        let result =
            self.node
                .block_on(io_deadline::run_for("peer exec request", timeout, async {
                    send_ctrl(&mut opened.send, &Ctrl::Exec { req }).await?;
                    recv_ctrl(&mut opened.recv).await
                }));
        let response = match result {
            Ok(Ctrl::ExecResp { result }) => result,
            Ok(Ctrl::ExecErr { msg }) => return Err(eio(msg)),
            Ok(_) => {
                let _ = self
                    .node
                    .invalidate_outgoing_session(&opened.session_key, opened.generation);
                return Err(eio("Peer sendet falsche Exec-Antwort"));
            }
            Err(error) => {
                io_deadline::abort(&mut opened.send, &mut opened.recv);
                let _ = self
                    .node
                    .invalidate_outgoing_session(&opened.session_key, opened.generation);
                return Err(error);
            }
        };
        crate::share::peer_telemetry::report_exec_success(
            &self.node.ev,
            format!(
                "Share-Exec: {} ms, code={:?}, timeout={}",
                started.elapsed().as_millis(),
                response.exit_code,
                response.timed_out
            ),
        );
        Ok(response)
    }
}
