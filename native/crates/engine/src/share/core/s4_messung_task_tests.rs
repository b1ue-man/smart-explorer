//! Messtests des Meilensteins M15 (E30): Einkern-Durchsatz je AEAD-Verfahren, Share-Durchsatz
//! ueber Loopback und Anteil mit Schwelle 10 %. Alle Tests sind `#[ignore]` und laufen nur
//! in der Suite `crates-task`, einzeln hintereinander (`--test-threads=1`), weil sie die
//! Rechenleistung des Runners messen. Aufruf: `cargo test --profile speed -p
//! smart_explorer_engine --lib s4_messung_task_ -- --ignored --nocapture --test-threads=1`.
//! Das Profil `speed` (`native/Cargo.toml`) misst die ausgelieferte Optimierung; mit dem
//! Entwicklungsprofil (opt-level 1) laeuft der QUIC-Code mehrfach langsamer, der Anteil
//! erschiene zu klein und die Schwelle wuerde zu Unrecht eingehalten.

use super::s4_messung_support::{
    aead_mib_s, aead_zeile, anteil_prozent, messung_zeile, negotiated_aead, platform,
    share_loopback_mib_s, Aead, THRESHOLD_PERCENT,
};

#[test]
#[ignore = "Messung; Suite crates-task"]
fn s4_messung_task_aead_aes_128_gcm() -> Result<(), String> {
    let mib_s = aead_mib_s(Aead::Aes128Gcm)?;
    println!("{}", aead_zeile(Aead::Aes128Gcm, mib_s));
    Ok(())
}

#[test]
#[ignore = "Messung; Suite crates-task"]
fn s4_messung_task_aead_aes_256_gcm() -> Result<(), String> {
    let mib_s = aead_mib_s(Aead::Aes256Gcm)?;
    println!("{}", aead_zeile(Aead::Aes256Gcm, mib_s));
    Ok(())
}

#[test]
#[ignore = "Messung; Suite crates-task"]
fn s4_messung_task_aead_chacha20_poly1305() -> Result<(), String> {
    let mib_s = aead_mib_s(Aead::ChaCha20Poly1305)?;
    println!("{}", aead_zeile(Aead::ChaCha20Poly1305, mib_s));
    Ok(())
}

/// Ausgehandeltes Verfahren, dessen AEAD-Durchsatz, Share-Durchsatz ueber Loopback und der
/// Anteil. Der Test besteht, wenn der Anteil unter 10 % liegt; ein Anteil von 10 % oder mehr
/// ist nach E30 ein Befund fuer den Klartextkanal und laesst den Test fehlschlagen.
#[test]
#[ignore = "Messung; Suite crates-task"]
fn s4_messung_task_share_loopback_anteil() -> Result<(), String> {
    let aead = negotiated_aead()?;
    let aead_mib = aead_mib_s(aead)?;
    let share_mib = share_loopback_mib_s()?;
    let anteil = anteil_prozent(share_mib, aead_mib);
    println!("{}", messung_zeile(&platform(), aead, aead_mib, share_mib));
    assert!(
        anteil < THRESHOLD_PERCENT,
        "anteil_prozent={:.1} liegt nicht unter {:.1} (E30: Befund fuer den Klartextkanal)",
        anteil,
        THRESHOLD_PERCENT
    );
    Ok(())
}
