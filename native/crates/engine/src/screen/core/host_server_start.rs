//! One W promise precedes metadata preparation. Only the final negotiated
//! binding can authorize a frame plan; unfinished native metadata stays pending.
use super::*;
use crate::screen::core::{
    capture_admission::SessionAdmission,
    capture_source_contract::PreparedSource,
    host_backend::ScreenSource,
    host_codec_adapters::{HostCodecPlan, HostContract, HostOutputPlan},
    pixel_convert::FrameSize,
    virtual_display_host::{self, SessionStart, Started},
};

pub(super) struct Ready {
    pub source: Box<dyn ScreenSource>,
    pub contract: Option<FrameContract>,
    pub choice: super::super::negotiate::CodecChoice,
    /// The 4:4:4 pairing for „Schärfe vor Flüssigkeit“ (B-32), if any.
    pub sharp: Option<super::super::negotiate::CodecChoice>,
    /// The pairings of the other video codecs, best first (merge W2).
    pub fallbacks: Vec<super::super::negotiate::CodecChoice>,
    pub admission: Arc<SessionAdmission>,
    /// The virtual display in the viewer's size (plan 2026-11-17 P5).
    pub display: Started,
}
pub(super) enum Failure {
    Capture(io::Error),
    Source(super::super::host_backend::SourceRefusal),
    /// No pairing; with the reason when a chosen codec has none (plan
    /// 2026-11-18, logged already).
    Encoder(Option<String>),
    Cancelled,
}
struct PreparedOutput {
    result: Result<Box<dyn PreparedSource>, super::super::host_backend::SourceRefusal>,
    _admission: Arc<SessionAdmission>,
}

impl Start<'_> {
    /// Refuses with `code`; `logged` when the cause has an entry already.
    pub(super) async fn refuse(&mut self, code: ScreenErrorCode, logged: bool) -> io::Result<()> {
        self.refuse_with(code, logged, None).await
    }

    /// `refuse` with the reason in words for the viewer (plan 2026-11-18:
    /// a chosen codec without a pairing names why).
    pub(super) async fn refuse_with(
        &mut self,
        code: ScreenErrorCode,
        logged: bool,
        message: Option<String>,
    ) -> io::Result<()> {
        if !logged {
            let reason = Some(code.key());
            log_event(
                self.context.audit,
                self.principal,
                self.mode,
                "session_refused",
                None,
                reason,
            );
        }
        let refused = HelloReply::Refused(match message {
            Some(message) => ScreenHelloError::with_message(code, message),
            None => ScreenHelloError::new(code),
        });
        send_hello_reply(&mut *self.send, &refused).await?;
        let _ = self.send.finish();
        let _ = tokio::time::timeout(CLOSE_GRACE, self.connection.closed()).await;
        Ok(())
    }

    /// `HELLO_OK` metadata from a blocking job; a stop of `handle` cancels it.
    pub(super) async fn metadata(
        &self,
        handle: Option<&SessionHandle>,
    ) -> io::Result<HostMetadata> {
        let mut query = HostMetadata::query(self.context, self.principal, self.mode);
        let stop = handle.map(SessionHandle::stop_signal);
        host_metadata::until(self.connection, stop, query.result()).await
    }

    /// Sends `HELLO_OK`; every write needs the rights of this mode.
    pub(super) async fn accepted(&mut self, ok: ScreenHelloOk) -> io::Result<()> {
        let reply = HelloReply::Accepted(ok);
        let (context, principal, mode) = (self.context, self.principal, self.mode);
        host_metadata::hello(context, principal, mode, self.connection, self.send, &reply).await
    }

    /// `HELLO_OK` of a streaming session with the answer to the viewer's
    /// sound offer (plan 2026-11-17 P9); an accepted offer attaches the
    /// capture link, which sends nothing before the viewer's `AUDIO` and
    /// ends with the connection.
    pub(super) async fn accepted_with_audio(&mut self, mut ok: ScreenHelloOk) -> io::Result<()> {
        let hello = self.hello;
        let link = self.context.backend.audio();
        ok.audio = super::super::audio_wire::answer(
            hello.audio,
            self.mode,
            hello.metadata_only,
            link.is_some(),
        );
        let answered = ok.audio.is_some();
        self.accepted(ok).await?;
        if let Some(link) = link.filter(|_| answered) {
            link.attach(self.connection);
        }
        Ok(())
    }

    /// A failed metadata step of the start: a stop or a leaving viewer ends
    /// it quietly like a cancelled preparation, a lost right refuses as
    /// such, any other failure (time limit, platform panic) refuses as a
    /// capture failure with its detail in the access log.
    pub(super) async fn unavailable(&mut self, error: io::Error) -> io::Result<()> {
        match error.kind() {
            io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted => Ok(()),
            io::ErrorKind::PermissionDenied => {
                self.refuse(ScreenErrorCode::PermissionDenied, false).await
            }
            _ => {
                let reason = error.to_string();
                let (audit, principal) = (self.context.audit, self.principal);
                log_event(
                    audit,
                    principal,
                    self.mode,
                    "session_refused",
                    None,
                    Some(&reason),
                );
                self.refuse(ScreenErrorCode::CaptureFailed, true).await
            }
        }
    }
}
pub(super) async fn prepare(
    context: &HostContext,
    principal: &ProvenPrincipal,
    hello: &ScreenClientHello,
    source_id: &SourceId,
    connection: &Connection,
    handle: &SessionHandle,
    metadata: &HostMetadata,
) -> Result<Ready, Failure> {
    let session = super::super::family_protocol::token().map_err(Failure::Capture)?;
    let mut resources = host_metadata::capture_resources(context, principal, hello.mode);
    let resources =
        host_metadata::until(connection, Some(handle.stop_signal()), resources.result())
            .await
            .map_err(|error| match error.kind() {
                io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted => Failure::Cancelled,
                // The same refusal as a preparation whose rights went away.
                io::ErrorKind::PermissionDenied => Failure::Source(
                    super::super::host_backend::SourceRefusal::new("source_permission_revoked"),
                ),
                _ => Failure::Capture(error),
            })?;
    let offered = resources.session_memory_ceiling.min(
        hello
            .frame_capabilities
            .map_or(super::super::frame_budget::MAX_WORKING_BYTES, |caps| {
                caps.frames.working_bytes
            }),
    );
    let admission = super::super::frame_memory::admit_session(
        resources.facts,
        resources.ceiling,
        offered,
        &session,
    )
    .map_err(Failure::Capture)?;
    let mut stop = handle.stop_signal();
    if stop.borrow().is_some() {
        return Err(Failure::Cancelled);
    }
    // P5: the display gets the viewer's size before the source is prepared.
    let display = virtual_display(context, principal, hello, source_id, handle, &session).await;
    let prepared = {
        let context = context.clone();
        let principal = principal.clone();
        let source = source_id.clone();
        let wanted = hello.display_size;
        let control = hello.mode == SessionMode::Control;
        let admission = admission.clone();
        let shown = display_shown(&display);
        let id = handle.id();
        tokio::task::spawn_blocking(move || {
            // P9: a Windows host attaches its virtual monitor beside the
            // physical one; the session shows the virtual one then.
            let source = match (&source, shown) {
                (SourceId::Monitor(current), Some(shown)) => {
                    super::super::set_display::display_source(
                        &context.backend.monitors(),
                        Some(current.as_str()),
                        shown,
                    )
                    .unwrap_or(source)
                }
                _ => source,
            };
            if let SourceId::Monitor(monitor) = &source {
                virtual_display_host::source_changed(&id, Some(monitor.clone()));
            }
            let result = super::super::family_listing::prepare(
                &context,
                &principal,
                &source,
                wanted,
                control,
                admission.clone(),
            );
            PreparedOutput {
                result,
                _admission: admission,
            }
        })
    };
    let prepared = tokio::select! {biased;
        _=connection.closed()=>return Err(Failure::Cancelled),
        _=stop.changed()=>return Err(Failure::Cancelled),
        result=prepared=>result.map_err(|error|Failure::Capture(io::Error::other(error)))?,
    };
    let prepared = prepared.result.map_err(Failure::Source)?;
    let info = prepared.description();
    info.capture_limits
        .validate_capture()
        .map_err(Failure::Capture)?;
    let mut host =
        FrameCapabilities::admitted(info.capture_limits.max_size, admission.working_bytes())
            .map_err(Failure::Capture)?;
    host.frames = host.frames.for_capture(info.capture_limits);
    host.hybrid_regions = info.supports_regions;
    let mut contract =
        FrameContract::negotiate(Some(host), hello.frame_capabilities).map_err(Failure::Capture)?;
    // HEVC and AV1 only from a viewer that named them in `decoders_v2`
    // (B-31); `HELLO_OK` named the host's in `encoders_v2` for it.
    let decoders = hello.all_decoders();
    // D6 (plan 2026-11-17 P3): the newer codecs first only when trusted.
    // Plan 2026-11-18 „Netzrate“: the viewer's choice, read from its
    // decoders (`preference_named_by`: a chosen codec names no tiles); a
    // chosen codec without a pairing ends the start with its reason.
    // Review 2026-11-19 (Codex 3/6): the hello's `codec_choice` names it
    // where the decoders cannot (one video codec under „Scharf (GPU)“).
    let preference = hello.codec_preference();
    // Review 2026-11-19: a viewer of codec negotiation v2 sends
    // `decoders_v2` (also empty); with no decoder at all it chose a codec
    // it cannot decode, read as „Automatisch" it got the tiles. A viewer
    // before v2 sends no list and keeps the tiles.
    let empty_choice = decoders.is_empty()
        && hello.decoders_v2.is_some()
        && preference == super::super::negotiate::CodecPreference::Auto;
    let negotiate = |contract| {
        if empty_choice {
            return Err(super::super::negotiate::ChosenUnavailable {
                preference: super::super::negotiate::CodecPreference::Auto,
                reason: super::super::negotiate::EMPTY_OFFER.into(),
            });
        }
        super::super::negotiate::negotiate_chosen(
            &metadata.encoders,
            &decoders,
            preference,
            contract,
            metadata.trust,
        )
    };
    let mut choices = negotiate(contract);
    // Without a video pairing under hybrid regions (they need decoders with
    // pixel access) the same offers without them first. Review 2026-11-19
    // (Codex 12): a result of the tiles alone counts as none; „Automatisch"
    // took the tiles while plain video worked. Without video there either,
    // a hybrid result of the tiles stays.
    let video = |result: &Result<Vec<super::super::negotiate::CodecChoice>, _>| {
        result
            .as_ref()
            .is_ok_and(|choices| choices.iter().any(|c| c.codec.is_video()))
    };
    if !video(&choices)
        && contract.is_some_and(|contract| contract.hybrid_regions)
        && decoders.iter().any(|offer| offer.codec.is_video())
    {
        let plain = contract.map(|mut contract| {
            contract.hybrid_regions = false;
            contract
        });
        let retry = negotiate(plain);
        if video(&retry) || !choices.as_ref().is_ok_and(|choices| !choices.is_empty()) {
            contract = plain;
            choices = retry;
        }
    }
    let choices = match choices {
        Ok(choices) => choices,
        Err(refused) => {
            log_event(
                context.audit,
                principal,
                hello.mode,
                "session_refused",
                None,
                Some(&refused.to_string()),
            );
            return Err(Failure::Encoder(Some(refused.to_string())));
        }
    };
    let choice = choices.first().copied().ok_or(Failure::Encoder(None))?;
    let sharp = super::super::negotiate::sharp_alternative(&choices, &choice);
    let fallbacks = super::super::host_pipeline::codec_fallbacks(&choices, &choice);
    admission
        .bind_contract(contract)
        .map_err(Failure::Capture)?;
    let limits = match contract {
        Some(contract) => contract.limits,
        None => super::super::frame_budget::legacy_limits(admission.working_bytes(), None)
            .map_err(Failure::Capture)?,
    };
    // This is the encoder envelope, not a fabricated native source size.
    let size = limits.fit(limits.max_size);
    let codec = HostCodecPlan::for_session(choice.codec, metadata.platform);
    let output = HostOutputPlan::new(
        contract.map_or(HostContract::Legacy, HostContract::Versioned),
        size,
        codec,
    )
    .map_err(Failure::Capture)?;
    let output = if contract.is_none() {
        output
            .with_legacy_limits(limits)
            .map_err(Failure::Capture)?
    } else {
        output
    };
    let allowed = {
        let context = context.clone();
        let principal = principal.clone();
        let control = hello.mode == SessionMode::Control;
        Arc::new(move || super::super::family_listing::admitted(&context, &principal, control))
    };
    let source =
        super::super::host_admission::pending(prepared, admission.clone(), output, allowed);
    Ok(Ready {
        source,
        contract,
        choice,
        sharp,
        fallbacks,
        admission,
        display,
    })
}

/// The virtual display for a viewer that sent a wish (plan 2026-11-17 P5):
/// the platform's display calls run on a worker thread. Only a monitor is
/// a display (a window, a window family and the test picture keep the
/// host's display). The driver setup with an elevation is offered only to a
/// viewer that may steer this screen; `capture` names the source
/// preparation of a Wayland host.
async fn virtual_display(
    context: &HostContext,
    principal: &ProvenPrincipal,
    hello: &ScreenClientHello,
    source: &SourceId,
    handle: &SessionHandle,
    capture: &str,
) -> Started {
    let monitor = matches!(source, SourceId::Monitor(_));
    if hello.virtual_display.is_none() || hello.metadata_only {
        return Started::default();
    }
    if !monitor {
        // „Wahl gilt“: the chosen window wins; the report says why.
        let report = super::super::virtual_display::VirtualDisplayReport::not_monitor(
            hello.virtual_display,
            hello.virtual_display_size,
            hello.mode,
            hello.metadata_only,
        );
        return Started {
            report,
            claim: None,
        };
    }
    let start = SessionStart {
        session: handle.id(),
        capture: capture.to_string(),
        mode: hello.mode,
        metadata_only: hello.metadata_only,
        wish: hello.virtual_display,
        fixed: hello.virtual_display_size,
        display_size: hello.display_size,
        may_setup: super::super::family_listing::admitted(context, principal, true),
        monitor: match source {
            SourceId::Monitor(id) => Some(id.clone()),
            _ => None,
        },
    };
    let driver = context.backend.virtual_display();
    tokio::task::spawn_blocking(move || virtual_display_host::start(driver, &start))
        .await
        .unwrap_or_default()
}

/// What the display shows the session after its opening decision: only
/// se's active display moves a session to another monitor (`None`: keep the
/// monitor the viewer chose).
fn display_shown(display: &Started) -> Option<super::super::set_display::Shown> {
    display
        .report
        .as_ref()
        .filter(|report| report.state == super::super::virtual_display::VirtualDisplayState::Active)
        .map(|_| super::super::set_display::Shown::Virtual)
}
