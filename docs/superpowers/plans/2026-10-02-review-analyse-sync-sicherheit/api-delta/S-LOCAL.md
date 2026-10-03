# S-LOCAL – API-Delta

Stand 2026-10-03. Keine neuen Cargo-Abhängigkeiten, keine lokale Ausführung.

## Gemeinsame private Storage-API

`native/src/support_dirs.rs` (pub(crate)):

```rust
ensure_private_dir(&Path) -> io::Result<()>
create_private_file(&Path) -> io::Result<File>
open_private_file(&Path) -> io::Result<File>
open_private_lock(&Path) -> io::Result<File>
secure_private_file(&File) -> io::Result<()>
read_private_text(&Path, max_bytes: u64) -> io::Result<String>
write_private_atomic(&Path, &[u8]) -> io::Result<()>
home_dir() -> Option<PathBuf>
```

- create_private_file ist exklusiv/nicht ersetzend, private Rechte vor Bytes.
  open_private_file/lock öffnen no-follow und prüfen Owner/Art/Hardlinks am Handle;
  eigene alte permissive Rechte werden vor Bytes enger. Lockfile ist read/write.
- Nur eigene private App-/Testeltern; nie direkt gemeinsamen `/tmp` als private Parent.
- write_private_atomic: zufällige `.se-private-<32 lowercase hex>.tmp`, private Erstellung,
  file-sync, Path-native V1-replace, Unix directory-sync. Kein neues Unicode-Erfordernis.
  Ein I/O-Fehler wird fortgeleitet; kein Erfolg behauptet, falls die letzte durable Bestätigung fehlt.
- home_dir bevorzugt HostConfig.home_dir, sonst USERPROFILE/HOME ohne Tempdir-Fallback.
  ShareProfiles-Lader erhalten `home_dir().map(|home| home.to_string_lossy().replace('\\', "/"))`;
  fehlende Home-Fakten bleiben sichtbar failclosed.
- V1-Reuse: local_access::DirectoryHandle::{open_root,create_private_child,create_file_new,
  secure_private,metadata,watch_path} und secure_private_handle(&File,bool). Windows-Implementierung
  bleibt V-LOCAL; eigene Adapter in creds/os/private_storage_{unix,windows}.rs.
- Identity-OS-Locks: acquire_until(&Path, Instant) teilt Gesamtfrist mit Prozessmutex.

## Uplink und LAN

- `Facility::RepairRequired(String)` ergänzt die bisherigen Varianten; is_available bleibt
  ausschließlich für Available wahr. `UplinkAdapter::cleanup_installation()` ist additive
  Default-Methode, OS-Adapter implementieren sichere Deaktivierung und Entfernung.
- `LanSettings`: additive serde(default)-Felder uplink_repair_requested_at:Option<i64> und
  uplink_cleanup_pending:bool. `UplinkView.repair_required:bool` ist ebenfalls defaulted.
  Persistenznamen und bisherige JSON-Felder bleiben erhalten.
- `PeerOnLink.authenticated:bool` verlangt authentisierte Link-Fakten. Die aktuelle Produktion
  setzt false für alle mDNS-Fakten; S09-LINK füllt dies erst aus echtem Iroh-Kanal/selektiertem Pfad.
- LAN v2: wechselnde HMAC-ID (32 hex), epoch/expiry, signierte Adressen/Ports/uplink, Node-Pin.
  Neue `LanProof`, `LanEvent::SeenAuthenticated{..}`, `LanPresence::announce_authenticated`.
  Legacy announce/Seen bleiben API-kompatibel als Dialhinweis. Kein Rechnername im Hostlabel.
- `share::lan_uplink_evidence::{publish,authorize}` ist private crate-interne Grenze. Snapshot
  ist bounded/private, Contact-Pins/Credentials/Signaturen werden erneut geprüft. authorize
  bleibt ohne echten S09-Kanal ablehnend; Snapshot stellt keine Autorität für ICS/NM dar.
- `net::run_uplink_helper_if_requested` erkennt --lan-uplink-cleanup für Windows/Linux;
  Windows Helperflag bleibt --lan-uplink-helper. NSIS verwendet vorhandenes se.exe-Einstiegsmodell.
- Windows-Aufgabe: Root-TaskPath `\`, Name `Smart Explorer LAN-Uplink <SID>`, admin-owned
  Kopie+Manifest in `%ProgramFiles%/Smart Explorer LAN-Uplink/<SID>`. Alte globale Aufgabe
  wird nur für ihren bestätigten aktuellen SID entfernt. Task-ACL wird bei COM-Registrierung
  gesetzt, nicht erst nachträglich. Manifestversion 2 bindet Hash/SID/Requestdirectory.
- Öffentliche Share-API für bewusste Firewall-Reparatur: lan_firewall_repair_available(),
  request_lan_firewall_repair(), start_lan_firewall_repair(), poll_lan_firewall_repair().
  GUI nutzt genau einen Hintergrundjob; System-PowerShell/UDP/private+domain, keine Startup-UAC.
- K1 Request 10: UncBackend::extensions delegiert an seinen bestehenden LocalBackend;
  Lease und persistierte UNC-/Verbindungsidentität bleiben erhalten.

## Externe API-Belege

Geprüft am 2026-10-03, ausschließlich Primärquellen. Rust try_lock liefert WouldBlock
für Konkurrenz und Error für echte I/O-Fehler; genutzt mit absolutem Deadline-Retry:
[Rust File::try_lock](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock),
[TryLockError](https://doc.rust-lang.org/std/fs/enum.TryLockError.html).

Windows-Elevation bekommt unveränderliches UTF-16LE EncodedCommand und absolut ausgewählten
System-Engine-Pfad; kein Script aus einem schreibbaren Stagingpfad:
[PowerShell EncodedCommand](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_powershell_exe?view=powershell-5.1#-encodedcommand-base64encodedcommand),
[Start-Process](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.management/start-process?view=powershell-7.5).
System-PowerShell lädt ausschließlich die per absolutem PSHOME-Pfad ausgewählten
Systemmodule; benutzerschreibbarer PSModulePath und Autoloading werden ausgeschlossen:
[Microsoft PSModulePath](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_psmodulepath?view=powershell-5.1),
[Microsoft module autoloading](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_modules?view=powershell-5.1).
Exact-handle ACL-Reparatur benötigt READ_CONTROL/WRITE_DAC und keinen Bytezugriff davor:
[Microsoft File Security](https://learn.microsoft.com/en-us/windows/win32/fileio/file-security-and-access-rights).

Die polkit-Regel schränkt den bekannten OS-Benutzer ausdrücklich auf lokale aktive Sitzung ein:
[polkit subject fields](https://polkit.pages.freedesktop.org/polkit/polkit.8.html).

Die exakten gelesenen, geänderten und neuen Dateien stehen vollständig in
[abnahme/S-LOCAL.md](../abnahme/S-LOCAL.md). Offene Owner-Anschlüsse stehen in
[anfragen/S-LOCAL.md](../anfragen/S-LOCAL.md). Keine eigene Graph-Neubau-/CI-/Release-Aktion.
