//! The encoder of the encoding stage (plan 2026-11-14 „Bildkette“ B-27):
//! opening it as `Worker::encoder_for` does, the OpenH264 switch and the
//! crash hook of the tests. Split from `host_worker_encode.rs`.
use super::*;
use crate::screen::core::codec_registry::TILES_NAME;

impl Encoding {
    pub(in crate::screen::os) fn new(opener: Arc<dyn EncoderOpener>) -> Self {
        Self {
            running: None,
            refused: Vec::new(),
            checked: Instant::now(),
            produced: 0,
            opener,
            sharp: false,
        }
    }

    /// Encodes `image` with the running or next stage of the rank (W4, B3):
    /// each failing stage goes onto `refused` once, asks the replacement for
    /// a key frame, and retries up to the stage count of `request`; a working
    /// instance that fails first gets one fresh instance of its own factory
    /// (`Running::failure_refuses`, review 2026-11-19 B1). The
    /// stage count is asked only after a failure (plan 2026-11-17
    /// „Windows-Bild“ P3: asking built the whole codec register, with every
    /// Media Foundation enumeration, for every picture).
    pub(in crate::screen::os) fn encode_with_fallback(
        &mut self,
        image: &Picture,
        mut request: std::borrow::Cow<'_, StageRequest>,
        grant: &CaptureGrant,
    ) -> io::Result<(
        crate::screen::core::host_backend::CapturedEncodedFrame,
        String,
        bool,
    )> {
        let mut max_attempts: Option<usize> = None;
        // Refused stages of this call; a fresh instance of the same factory
        // (B1) is no stage of the rank and does not count.
        let mut attempt = 0;
        let mut fresh: Option<String> = None;
        loop {
            // Measurement of 2026-11-18: every encoder failure with its
            // reason into the host log (this process's standard error).
            let running = self.encoder_for(&request, grant).inspect_err(|error| {
                eprintln!(
                    "screen encoder: no {} encoder opens: {error}",
                    request.choice.codec.key()
                )
            })?;
            // The registry opens the next stage when that factory no longer opens.
            running.replaces_failed |= fresh.take().as_ref() == Some(&running.factory);
            // Station `convert`: the successful encoder's own conversion call (r1).
            crate::screen::core::convert_clock::arm();
            let encoded = running.encoder.encode_picture(image, &request);
            match encoded.and_then(|frame| keyed(frame, &request)) {
                Ok(frame) => {
                    running.delivered += 1;
                    return Ok((frame, running.encoder.name(), running.hardware));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let _ = crate::screen::core::convert_clock::take();
                    return Err(error);
                }
                Err(error) => {
                    let _ = crate::screen::core::convert_clock::take();
                    let factory = running.factory.clone();
                    let refuses = running.failure_refuses();
                    eprintln!("screen encoder: {} failed: {error}", running.encoder.name());
                    self.running = None;
                    if factory == crate::screen::core::codec_registry::TILES_NAME {
                        return Err(error);
                    }
                    request.to_mut().keyframe = true;
                    if !refuses {
                        eprintln!(
                            "screen encoder: a fresh {factory} encoder with a key frame after its first failure"
                        );
                        fresh = Some(factory);
                        continue;
                    }
                    if !self.refused.contains(&factory) {
                        self.refused.push(factory);
                    }
                    attempt += 1;
                    let opener = &self.opener;
                    let stages =
                        *max_attempts.get_or_insert_with(|| opener.stage_count(&request).max(1));
                    if attempt >= stages {
                        return Err(error);
                    }
                }
            }
        }
    }

    pub(super) fn needs_refresh(&self) -> bool {
        self.running
            .as_ref()
            .is_some_and(|running| running.encoder.needs_refresh())
    }

    /// Once a second: a running OpenH264 encoder goes when the user
    /// switched OpenH264 off (as `Worker::check_openh264`).
    pub(super) fn check_openh264(&mut self) {
        if self.checked.elapsed() < OPENH264_CHECK {
            return;
        }
        self.checked = Instant::now();
        let openh264 = self
            .running
            .as_ref()
            .is_some_and(|running| running.encoder.name() == OPENH264);
        if openh264 && !openh264_switched_on() {
            self.running = None;
        }
    }

    #[cfg(debug_assertions)]
    pub(super) fn crash_hook(&mut self) {
        self.produced += 1;
        let limit = std::env::var(CRASH_AFTER_ENV)
            .ok()
            .and_then(|value| value.parse::<u64>().ok());
        if limit.is_some_and(|limit| self.produced > limit) {
            std::process::abort();
        }
    }

    #[cfg(not(debug_assertions))]
    pub(super) fn crash_hook(&mut self) {
        self.produced += 1;
    }

    /// Opens the encoder of the rank for `request`. Review 2026-11-19 C16:
    /// „Scharf (GPU)“ takes no software encoder; one the rank offers is
    /// passed over for this opening only (it stays usable when the stream
    /// leaves sharp mode), and without a GPU encoder left the failure names
    /// the GPU encoders that gave way.
    fn open_for(
        &self,
        request: &StageRequest,
        grant: &CaptureGrant,
    ) -> io::Result<crate::screen::core::codec_registry::OpenedEncoder> {
        let mut passed = self.refused.clone();
        loop {
            let opened = match self.opener.open(request, &passed, grant.clone()) {
                Err(error)
                    if passed.len() > self.refused.len()
                        && error.kind() != io::ErrorKind::WouldBlock =>
                {
                    let (codec, failed) = (request.choice.codec.key(), self.refused.join(", "));
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        format!(
                            "Scharf (GPU): no GPU encoder of {codec} is left \
                             (failed: [{failed}]; software passed over: {error})"
                        ),
                    ));
                }
                opened => opened?,
            };
            if !self.sharp || opened.hardware || opened.factory == TILES_NAME {
                return Ok(opened);
            }
            eprintln!(
                "screen encoder: Scharf (GPU) passes over the software encoder {}",
                opened.factory
            );
            passed.push(opened.factory);
        }
    }

    /// The encoder for `request`, as `Worker::encoder_for` chooses it.
    pub(super) fn encoder_for(
        &mut self,
        request: &StageRequest,
        grant: &CaptureGrant,
    ) -> io::Result<&mut Running> {
        let book = grant.book_identity();
        let current = self.running.as_ref().is_some_and(|running| {
            running.book == book
                && running.epoch == request.encoder_epoch
                && running.choice == request.choice
                && running.frame_contract == request.frame_contract
                && running.encoder.size() == request.size
        });
        if !current {
            if let Some(running) = &self.running {
                eprintln!(
                    "screen encoder: opens anew ({})",
                    reopen_reason(running, request, book)
                );
            }
            self.running = None;
            let opened = self.open_for(request, grant)?;
            self.running = Some(Running {
                encoder: opened.encoder,
                factory: opened.factory,
                hardware: opened.hardware,
                epoch: request.encoder_epoch,
                choice: request.choice,
                frame_contract: request.frame_contract,
                book,
                delivered: 0,
                replaces_failed: false,
            });
        }
        self.running
            .as_mut()
            .ok_or_else(|| io::Error::other("no screen encoder"))
    }
}
