//! No network, peer transport or image decoding belongs in this process.
use super::{super::platform, child::HelperLink, denied, installation, nonce, now};
use crate::login::core::{
    authority::Authority,
    model::Operation,
    wire::{self, Reply, Request, PIXELS},
};
use std::{
    io::{self, Read, Write},
    time::{Duration, Instant},
};

pub(crate) fn serve(stream: &mut (impl Read + Write), boot: &str) -> io::Result<()> {
    let Request::Hello {
        peer_id,
        target_user,
    } = wire::receive(stream)?
    else {
        return Err(denied("login hello required"));
    };
    if peer_id.is_empty()
        || peer_id.len() > 256
        || target_user.is_empty()
        || target_user.len() > 256
    {
        return Err(denied("invalid login identity length"));
    }
    let install = installation::load()?;
    let mut evidence = super::host_status::SessionEvidence::new(boot, &install.policy);
    let binding = platform::binding(&install)?;
    let mut sequence = 1u64;
    let mut authority = Authority::new(
        &install.policy,
        boot.into(),
        binding,
        peer_id.clone(),
        target_user.clone(),
        nonce()?,
        sequence,
        now()?,
    );
    let mut first = Reply::state("challenge");
    first.challenge = Some(authority.challenge().clone());
    wire::send(stream, &first)?;
    let mut helper: Option<HelperLink> = None;
    let mut pinned_binding = None;
    let mut pinned_generation = String::new();
    loop {
        let started = Instant::now();
        let Request::Signed(request) = wire::receive(stream)? else {
            return Err(denied("signed login operation required"));
        };
        let install = installation::load()?;
        let actual = platform::binding(&install)?;
        if started.elapsed() > Duration::from_secs(30)
            || pinned_binding.as_ref().is_some_and(|old| old != &actual)
            || (!pinned_generation.is_empty() && pinned_generation != install.manifest.generation)
        {
            helper.take();
            wire::send(
                stream,
                &Reply::refused("login session or release generation changed; reconnect"),
            )?;
            return Ok(());
        }
        if let Err(reason) = authority.authorize(&request, &install.policy, &actual, now()?) {
            helper.take();
            wire::send(stream, &Reply::refused_with(reason.state, reason.reason))?;
            return Ok(());
        }
        let stop = matches!(request.operation, Operation::Stop);
        let had_helper = helper.is_some();
        let answer = match request.operation {
            Operation::Start => {
                helper.take();
                let mut link = HelperLink::start(&install, &actual, false)?;
                let reply = link.ready()?;
                if reply.state == "ready" && reply.binding.as_ref() == Some(&actual) {
                    helper = Some(link);
                    pinned_binding = Some(actual.clone());
                    pinned_generation = install.manifest.generation.clone();
                    Ok((reply, None))
                } else {
                    Ok((Reply::refused_with("unavailable", "login helper did not confirm its exact authorized session/desktop binding"), None))
                }
            }
            Operation::Stop => {
                helper.take();
                Ok((Reply::state("stopped"), None))
            }
            Operation::SecureAttention if helper.is_some() => {
                // The broker cannot inspect the helper's capture geometry.
                // Release through that live context before the privileged action;
                // an ended geometry lease must terminate this session too.
                helper
                    .as_mut()
                    .ok_or_else(|| denied("login helper is absent"))?
                    .ask(Operation::ReleaseInputs)
                    .and_then(|_| {
                        platform::secure_attention(&actual)
                            .map(|()| (Reply::state("secure_attention_requested"), None))
                            .or_else(|error| {
                                Ok((Reply::refused_with("policy_blocked", error), None))
                            })
                    })
            }
            operation => helper
                .as_mut()
                .ok_or_else(|| denied("start the authorized login helper first"))
                .and_then(|helper| helper.ask(operation)),
        };
        let (mut reply, pixels) = match answer {
            Ok(value) => value,
            Err(error) => {
                helper.take();
                let refused = Reply::refused(error);
                // A dead capture context cannot issue a fresh challenge with
                // the old desktop/frame identity. A new viewer must start it.
                if had_helper {
                    wire::send(stream, &refused)?;
                    return Ok(());
                }
                (refused, None)
            }
        };
        if stop {
            wire::send(stream, &reply)?;
            return Ok(());
        }
        // Revalidate the boundary after the effect and before publishing pixels.
        let fresh = installation::load()?;
        let latest = platform::binding(&fresh)?;
        if fresh.policy.revision != install.policy.revision
            || fresh.manifest.generation != install.manifest.generation
            || latest != actual
        {
            helper.take();
            wire::send(
                stream,
                &Reply::refused("login policy, release or desktop changed; reconnect"),
            )?;
            return Ok(());
        }
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| denied("login sequence exhausted"))?;
        authority = Authority::new(
            &fresh.policy,
            boot.into(),
            latest,
            peer_id.clone(),
            target_user.clone(),
            nonce()?,
            sequence,
            now()?,
        );
        reply.challenge = Some(authority.challenge().clone());
        // Private inherited grant identifiers never enter the gateway wire.
        reply.capture_id = None;
        evidence.update(helper.as_ref().map(HelperLink::process_id), &fresh.policy)?;
        wire::send(stream, &reply)?;
        if let Some(pixels) = pixels {
            wire::write_frame(stream, PIXELS, pixels.as_slice())?;
        }
    }
}
