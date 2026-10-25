//! UDisks2 invalidates inventory; it never mounts a device or changes permissions.
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use crate::app::drive_events::Invalidate;
use futures_util::StreamExt;

pub(crate) fn start_drive_events(invalidate: Invalidate, stop: Arc<AtomicBool>) {
    let spawned = std::thread::Builder::new().name("udisks-events".into()).spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(error) => { eprintln!("UDisks2-Ereignisse: {error}"); return; }
        };
        runtime.block_on(async move {
            while !stop.load(Ordering::Acquire) {
                let connected = tokio::time::timeout(Duration::from_secs(3), async {
                    let connection = zbus::Connection::system().await?;
                    let rule = zbus::MatchRule::builder()
                        .msg_type(zbus::message::Type::Signal)
                        .sender("org.freedesktop.UDisks2")?
                        .path_namespace("/org/freedesktop/UDisks2")?
                        .build();
                    let stream = zbus::MessageStream::for_match_rule(rule, &connection, Some(32)).await?;
                    Ok::<_, zbus::Error>((connection, stream))
                }).await;
                if let Ok(Ok((_connection, mut stream))) = connected {
                    invalidate();
                    while !stop.load(Ordering::Acquire) {
                        match tokio::time::timeout(Duration::from_millis(500), stream.next()).await {
                            Ok(Some(Ok(_))) => invalidate(),
                            Ok(_) => break,
                            Err(_) => {}
                        }
                    }
                }
                // Inventory polling remains active while the system bus is unavailable.
                for _ in 0..10 {
                    if stop.load(Ordering::Acquire) { return; }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        });
    });
    if let Err(error) = spawned { eprintln!("UDisks2-Ereignisse: {error}"); }
}
