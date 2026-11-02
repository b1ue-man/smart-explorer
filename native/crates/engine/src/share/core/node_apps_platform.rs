//! Local Android result/consent callbacks retaining the original node owner.
use super::*;

fn platform_node() -> &'static Mutex<Weak<ShareIrohNode>> {
    static NODE: std::sync::OnceLock<Mutex<Weak<ShareIrohNode>>> = std::sync::OnceLock::new();
    NODE.get_or_init(|| Mutex::new(Weak::new()))
}
pub(super) fn install_platform(node: Weak<ShareIrohNode>) -> io::Result<()> {
    *platform_node()
        .lock()
        .map_err(|_| eio("app callback owner unavailable"))? = node;
    crate::apps::os::host::install_platform_hooks(platform_result, platform_consent)
}
fn platform_result(
    request: LaunchRequest,
    dispatch: String,
    result: crate::apps::os::shared::platform::DispatchReceipt,
) {
    let Some(node) = platform_node()
        .lock()
        .ok()
        .and_then(|current| current.upgrade())
    else {
        return;
    };
    let context = node
        .apps
        .pending
        .lock()
        .ok()
        .and_then(|pending| pending.get(request.request_id.as_str()).cloned());
    let Some(context) = context.filter(|context| context.request == request) else {
        return;
    };
    let principal = context.principal;
    let original = context.request;
    // The JNI call must return immediately; the host owns its serialization
    // guard while Kotlin is delivering this result.
    let _ = std::thread::Builder::new()
        .name("app-local-result".into())
        .spawn(move || {
            if let Ok(host) = node.apps.host(&principal) {
                if host
                    .bind_peer_owner(
                        &original.request_id,
                        &owner_key(&principal).unwrap_or_default(),
                        false,
                    )
                    .unwrap_or(false)
                {
                    if let Ok(receipt) = host.note_dispatch_result(&original, &dispatch, result) {
                        if receipt.dispatch_id == dispatch
                            && !matches!(
                                receipt.state,
                                crate::apps::LaunchState::ConsentRequired
                                    | crate::apps::LaunchState::OutcomeUnknown
                            )
                        {
                            if let Ok(mut pending) = node.apps.pending.lock() {
                                pending.remove(original.request_id.as_str());
                            }
                        }
                    }
                }
            }
        });
}
fn platform_consent(request: LaunchRequest) {
    let Some(node) = platform_node()
        .lock()
        .ok()
        .and_then(|current| current.upgrade())
    else {
        return;
    };
    let context = node
        .apps
        .pending
        .lock()
        .ok()
        .and_then(|pending| pending.get(request.request_id.as_str()).cloned());
    let Some(context) = context.filter(|context| context.request == request) else {
        crate::apps::os::host::end_platform_consent(
            &request,
            "original_app_peer_context_unavailable",
        );
        return;
    };
    let principal = context.principal;
    let original = context.request;
    let original_access = context.access;
    let runtime = node.rt.clone();
    runtime.spawn(async move {
        let result = async {
            if original_access
                .as_ref()
                .is_none_or(|access| access.reason().is_some())
            {
                return Err(eio("original app admission was revoked or expired"));
            }
            let host = node.apps.host(&principal)?;
            let catalog = tokio::task::spawn_blocking(move || host.catalog())
                .await
                .map_err(eio)??;
            let access = access_request(&original, &catalog)?;
            // The original contextual Ask approval remains represented by its
            // retained AccessHandle. Recheck it and the exact current policy;
            // a local Android BAL confirmation never creates a new E4 grant.
            {
                let state = node
                    .auth
                    .lock()
                    .map_err(|_| eio("authorization unavailable"))?;
                if request_for(&state, &principal, &access, now_secs()).decision
                    == CapabilityDecision::Off
                {
                    return Err(eio("app grant ended while awaiting local consent"));
                }
            }
            let node = node.clone();
            let principal = principal.clone();
            let original = original.clone();
            tokio::task::spawn_blocking(move || {
                node.run_app(&principal, &original, true, None, None)
            })
            .await
            .map_err(eio)?
        }
        .await;
        if let Err(error) = result {
            if let Ok(host) = node.apps.host(&principal) {
                if let Ok(Some(receipt)) = host.receipt(&original) {
                    let _ = host.note_dispatch_result(
                        &original,
                        &receipt.dispatch_id,
                        crate::apps::os::shared::platform::DispatchReceipt {
                            state: crate::apps::LaunchState::Denied,
                            reason: format!("local_app_consent_refused: {error}"),
                            pid: None,
                        },
                    );
                }
            }
            crate::apps::os::host::end_platform_consent(&original, &error.to_string());
        } else if let Ok(receipt) = result {
            if matches!(
                receipt.state,
                crate::apps::LaunchState::Denied | crate::apps::LaunchState::Unsupported
            ) {
                crate::apps::os::host::end_platform_consent(&original, &receipt.reason);
            }
        }
    });
}
