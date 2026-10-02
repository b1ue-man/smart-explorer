# Anfragen von S-SIGNAL

Stand: 2026-10-02. Je Eintrag: Datei, Stelle, Änderung, Grund. Der Code von S-SIGNAL ist bereits gegen
diese Änderungen geschrieben (lokale Platzhalter nur, wo vermerkt).

## A1 (Orchestrator) – `share-server/Cargo.toml` + `share-server/Cargo.lock`

Alle Versionen liegen schon im `share-server/Cargo.lock` (statisch mit `cargo metadata --offline` auflösbar):

```toml
[dependencies]
# ersetzt `iroh-base = "1.0.0"`: PublicKey/Signature für die Schlüssel-Anmeldung ausdrücklich
iroh-base = { version = "1.0.0", features = ["key"] }
# TLS für Signaling (wss) und Relay (https); ring statt aws-lc-rs, keine Standard-Features
rustls = { version = "0.23.41", default-features = false, features = ["ring", "std", "tls12"] }
# SHA-256 (Nachweis-Hashes, Anmelde-Digest, Zertifikats-Fingerabdruck) und Server-Nonces
ring = "0.17"

[dev-dependencies]
# selbstsignierte Testzertifikate für die TLS-Tests (liegt als 0.14.8 schon im Lock, über iroh-relay)
rcgen = "0.14"
```

Grund: FC4 (Server spricht selbst TLS, Anmeldung mit Geräteschlüssel), recherche E8,
`docs/refs/share-server-tls-auth.md` „Zusammenfassung der Abhängigkeiten“ (statt hmac/hkdf/sha2/blake3/rand
genügt `ring`, der Server prüft nur SHA-256-Hashes und erzeugt Nonces).
