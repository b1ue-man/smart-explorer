//! The host session's side of the bandwidth probe and the „Übertragung“
//! (`bandwidth_probe.rs`, `delivery.rs`): the viewer's delivery from its
//! hello (absent: `Delivery::default()`) and from `SET_DELIVERY` goes to the
//! pipeline; the probes the schedule asks for start on the session's
//! connection; an accepted measurement goes to the pipeline
//! (`Pipeline::set_measured_bps`, which logs it) and, with its age and the
//! delivery, into `STATS` (`ChainReport::measured_kbps`, `measured_age_ms`,
//! `delivery`). A viewer that did not announce `bandwidth_probe` gets no
//! probe stream; its host keeps the stream's table rate.
use std::io;
use std::time::Instant;

use iroh::endpoint::Connection;

use super::super::delivery::Delivery;
use super::super::gop_stream::{now_us, path_of, picture_session};
use super::super::host_pipeline::PipelineActor;
use super::super::wire_control::{HostStats, ViewerFrame};
use super::super::wire_hello::ScreenClientHello;
use super::{ProbeFacts, ProbeHead, ProbeResult, ProbeSchedule, BANDWIDTH_PROBE_V1};

/// One line in the screen host log.
fn note(line: &str) {
    let line = format!("screen probe: {line}");
    // The unit tests write no file of the account they run under.
    #[cfg(not(test))]
    crate::screen::os::host_worker_client::host_log::note(None, &line);
    #[cfg(test)]
    let _ = line;
}

/// The line facts of one host session; the default is a session without
/// probes under the default delivery.
#[derive(Default)]
pub struct LineLink {
    /// The viewer reads probe streams (`ScreenClientHello::bandwidth_probe`).
    probes: bool,
    /// The session streams the full view; previews are not measured.
    full: bool,
    delivery: Delivery,
    schedule: ProbeSchedule,
    /// The writer of the last probe stream; it ends with its duration.
    writer: Option<tokio::task::JoinHandle<io::Result<u64>>>,
    /// The last accepted measurement in bit/s and when it arrived.
    measured: Option<(u64, Instant)>,
}

impl LineLink {
    /// The session `hello` opened: its „Übertragung“ and whether its viewer
    /// reads probe streams.
    pub fn for_hello(hello: &ScreenClientHello) -> Self {
        Self {
            probes: hello.bandwidth_probe == Some(BANDWIDTH_PROBE_V1) && !hello.metadata_only,
            full: picture_session(hello.mode, hello.metadata_only),
            delivery: hello.delivery.unwrap_or_default(),
            schedule: ProbeSchedule::default(),
            writer: None,
            measured: None,
        }
    }

    pub fn delivery(&self) -> Delivery {
        self.delivery
    }

    /// The last accepted measurement in bit/s.
    pub fn measured_bps(&self) -> Option<u64> {
        self.measured.map(|(bps, _)| bps)
    }

    /// The id of the probe waiting for its answer.
    pub fn running(&self) -> Option<u64> {
        self.schedule.running()
    }

    /// `SET_MODE` raised the session to the full view (`true`) or lowered it
    /// to a preview.
    pub fn set_full(&mut self, full: bool) {
        self.full = full;
    }

    /// `SET_DELIVERY`, `REMEASURE` and `PROBE_RESULT`; another frame is
    /// refused.
    pub fn on_frame(
        &mut self,
        frame: ViewerFrame,
        pipeline: &mut PipelineActor,
        connection: &Connection,
    ) -> io::Result<()> {
        match frame {
            ViewerFrame::SetDelivery(delivery) => {
                self.delivery = delivery;
                pipeline.set_delivery(delivery);
            }
            ViewerFrame::Remeasure => {
                self.schedule.request();
                self.turn(pipeline, connection);
            }
            ViewerFrame::ProbeResult(result) => self.on_result(result, pipeline),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "not a frame of the line",
                ))
            }
        }
        Ok(())
    }

    fn on_result(&mut self, result: ProbeResult, pipeline: &mut PipelineActor) {
        let id = result.probe_id;
        if !self.schedule.on_result(id) {
            note(&format!(
                "answer to probe {id} came late or unasked: ignored"
            ));
            return;
        }
        let ms = result.duration_us / 1000;
        if result.bps == 0 {
            note(&format!(
                "probe {id}: no complete window in {ms} ms ({} bytes): the rate stays",
                result.bytes
            ));
            return;
        }
        note(&format!(
            "probe {id}: {} kbit/s at most per window, {} bytes in {ms} ms",
            result.bps / 1000,
            result.bytes
        ));
        pipeline.set_measured_bps(result.bps);
        self.measured = Some((result.bps, Instant::now()));
    }

    /// Once a second and on `REMEASURE`: drops an unanswered probe and
    /// starts the one the schedule asks for.
    pub fn turn(&mut self, pipeline: &PipelineActor, connection: &Connection) {
        if !self.probes {
            return;
        }
        let now = now_us();
        if let Some(id) = self.schedule.expire(now) {
            note(&format!("probe {id}: no answer within 3 s, dropped"));
        }
        let route = crate::share::route_monitor::snapshot(connection);
        let facts = ProbeFacts {
            now_us: now,
            route: route
                .as_ref()
                .map(|route| (route.generation, route.pause_epoch, route.class)),
            sendable: self.full && route.as_ref().is_none_or(|route| route.application_allowed),
            delivery: self.delivery,
            line: pipeline.line_facts(),
        };
        let Some((id, reason)) = self.schedule.due(&facts) else {
            return;
        };
        let (_, srtt_us) = path_of(connection);
        let head = ProbeHead::for_rtt(id, srtt_us);
        note(&format!(
            "probe {id} ({}) for {} ms",
            reason.key(),
            head.duration_ms
        ));
        // The writer before ended with its duration (at most 1.5 s, and the
        // schedule waits for its answer or 3 s); an abort resets its stream.
        let writer = tokio::spawn(super::send_probe(connection.clone(), head));
        if let Some(previous) = self.writer.replace(writer) {
            previous.abort();
        }
    }

    /// The delivery and the last measurement for `STATS`.
    pub fn report(&self, stats: &mut HostStats) {
        let Some(chain) = stats.chain.as_mut() else {
            return;
        };
        chain.delivery = Some(self.delivery);
        chain.measured_kbps = self.measured.map(|(bps, _)| bps / 1000);
        chain.measured_age_ms = self
            .measured
            .map(|(_, at)| u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX));
    }

    /// The session's second: `turn`, then `report`.
    pub fn second(
        &mut self,
        pipeline: &PipelineActor,
        connection: &Connection,
        stats: &mut HostStats,
    ) {
        self.turn(pipeline, connection);
        self.report(stats);
    }
}

impl Drop for LineLink {
    /// A session that ends takes its probe stream along (reset, not sent on).
    fn drop(&mut self) {
        if let Some(writer) = self.writer.take() {
            writer.abort();
        }
    }
}
