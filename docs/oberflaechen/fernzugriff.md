# Maschinenanmeldung, TCP, RDP, Programme und Fensterfamilien

Quellstand: 2026-10-26, laufender Gesamtkandidat aus
[dem Batchplan](../plaene/2026-10-24-batches/batches.md). Die folgenden Anschlüsse
liegen im Quelltext vor. Die gemeinsame Remote-Suite und Veröffentlichung stehen
aus. `N/` bedeutet `native/src/`, `A/` bedeutet
`android/app/src/main/java/app/smartexplorer/android/`.

## Erreichbare Oberflächen

| Fähigkeit | CLI und MCP | Desktop | Android und Bridge | Gemeinsame Grenze |
|---|---|---|---|---|
| Maschinenanmeldung | `se login status/provision/target/open/sessions/session/close` (`N/cli/login.rs:10`); MCP `login` (`N/cli/mcp/catalog.rs:203`) | Geräteseite → Maschinenanmeldung (`N/app/core/share_login_ui.rs:13`), eigener bildgebundener Renderer `login_viewer.rs` | `LoginTargetDialog` und `LoginViewScreen` (`A/ui/share/ShareDialogHost.kt:107`); `login.viewOpen`, Status, Fokus, Presented, Input, SAS, Release, Close (`N/mobile/os/shared/domains/login_view.rs:26`) | Explizites öffentliches Maschinenziel, bestätigter Maschinenpin, administrativer Grant und SID/UID. Ein gewöhnlicher Gerätegrant erteilt diese Rechte nicht. |
| TCP-Weiterleitung | `se tcp start/list/status/close` (`N/cli/tcp.rs:25`); MCP `tcp_forward` (`N/cli/mcp/catalog.rs:200`) | Geräteseite → TCP (`N/app/core/share_tcp_ui.rs:11`) | `TcpDialog`, `TcpApi`; `tcp.*` (`N/mobile/os/shared/domains/mod.rs:133`) | Gepinnte Direct-/Roomidentität, konkretes Ziel, getrennte IP-Autorisierung, endliche Lease und ausdrückliche Bestätigung des lokalen Loopbackzugriffs. |
| Vorhandener RDP-Dienst | `se rdp local-probe/probe/clients/open/list/status/close` (`N/cli/rdp.rs:9`); MCP `rdp` (`N/cli/mcp/catalog.rs:202`) | Geräteseite → Dienst, Client, Zertifikatpolicy, Sitzungswirkung und optional veröffentlichte RemoteApp (`N/app/core/share_device_page_ui.rs:200`) | `RdpDialog` und `rdp.*` zeigen authentifizierten Hostbestand und lokale Clientfähigkeit (`N/mobile/os/shared/domains/mod.rs:136`) | Nur der authentifizierte Kanal erzeugt `AuthenticatedProbe`; frei übergebenes JSON ist kein Hostbeleg. Start braucht aktuellen Service-/Zertifikatbeleg und die angezeigten Bestätigungen. |
| Installierte Anwendung starten | `se apps`, MCP `apps` (`N/cli/mcp/catalog.rs:201`) | Gerätekatalog und lokale Programmfreigabe (`N/app/core/share_apps_ui.rs`, `share_device_page_ui.rs:201`) | Appkatalog/-dialog und `apps.*` (`N/mobile/os/shared/domains/mod.rs:135`) | Katalogkennung, angemeldeter Hostnutzer und Sitzung; bestätigter oder ausdrücklich unklarer Startausgang. Keine freie Shellausführung aus dem Appkatalog. |
| Fensterfamilie ansehen/steuern | `se screen families` (`N/cli/screen/families.rs:11`) und Fensterbetrachter; MCP `screen_window` (`N/cli/mcp/catalog.rs:198`) | Familienauswahl an derselben Gerätebeziehung (`N/app/core/family_device_ui.rs:5`), eigener Familienbetrachter | `FamilyPicker`, `FamilyScreen` (`A/ui/share/ShareDialogHost.kt:102`); `screen.families` und `screen.viewFamily*` | Familiennonce, Mitgliedsgeneration, tatsächliche Layoutrevision und bestätigtes Fokusmitglied. Close/Resize sind Anfragen an die Anwendung; der folgende Snapshot belegt das Ergebnis. |

Die Desktop-Geräteseite verbindet dieselben Fähigkeiten für Kontakte, direkte
Grants und Raummitglieder (`share_device_page_ui.rs:200`, `:378`, `:457`).
CLI, MCP und Desktop verwenden die registrierten Daemon-IPC-Wege; die mobile
Bridge verwendet denselben Kern und dieselben typisierten Rechtegrenzen.
Registrierungen: `N/daemon/mod.rs`, `N/cli/mcp/server.rs:146` und
`N/mobile/os/shared/domains/mod.rs:133`.

Android führt `OpenApps`, `OpenScreen`, `ShowScreenGrid` und
`ShowShareRequests` auch aus den Einstellungsbereichen Darstellung,
Fehlerprotokoll und Info zur Geräteansicht. Die Einstellungsnavigation erhält
die Anfrage bis zur Bearbeitung durch `ShareScreen`; Geräteidentität und
Programmziel bleiben erhalten (`A/ui/settings/SettingsNavigation.kt`,
`SettingsScreen.kt`, `A/ui/AppRoot.kt`). Der getrennte Desktopflächenprozess
registriert den Familienbetrachter selbst. Die eframe-Fläche und die
Wayland-Layer-Fläche zeichnen Programmauswahl und Fehler beim Öffnen einer
Fensterfamilie über dieselbe Funktion `show_surface`
(`N/app/core/desktop_surface_app.rs`, `desktop_surface_grid_ui.rs`).

## Besitz, Darstellung und Beendigung

Loginbilder laufen binär zum Renderer. CLI/MCP erhalten Status und Bildmetadaten;
sie transportieren keine Loginpixel in JSON. Eingaben benötigen den Beleg des
tatsächlich dargestellten Bildes und die aktuelle Präsentationsepoche.
Ein erkannter Sitzungs-, Desktop- oder Geometriewechsel beendet den alten
Login-Kontext. Gehaltene Eingaben werden freigegeben. Wiederaufnahme benötigt
eine neue ausdrückliche Sitzung und einen neuen Darstellungsbeleg.
Quellen: `N/login/core/viewer_guard.rs`, `N/login/os/shared/{helper,broker,viewer}.rs`.

Der native RDP-Besitzer hält Clientprozess und TCP-Lease gemeinsam. Beenden,
Rechteentzug und Appupdate schließen beide; ein fehlgeschlagener Abschluss
bleibt sichtbar und wiederholbar. CLI-/MCP-/Bridge-`list/status` zeigen den
gemeinsamen TCP-Leasebestand und damit auch reine TCP-Tunnel. Die Desktopliste
zeigt die tatsächlich im Prozess besessenen RDP-Clients. Quellen:
`N/rdp/os/shared/session.rs`, `N/app/os/shared/rdp_exit_gate.rs` und
`N/cli/mcp/rdp_shutdown.rs`.

## Plattformgrenzen und Administration

- Der privilegierte Login-Dienst wird ausdrücklich mit dem passenden
  veröffentlichten Administrationsskript eingerichtet. Die normale
  Appinstallation aktiviert ihn nicht. Installation, Aktualisierung und
  Entfernung stehen in [RELEASING](../RELEASING.md#optional-login-services).
- Android ist Loginbetrachter. Der Quellstand bietet dort keinen privilegierten
  Loginhost und keinen belegten nativen RDP-Clientadapter an; die Oberfläche
  zeigt den konkreten Fähigkeitsgrund. Es entsteht kein Ersatzstart über eine URI.
- RDP verwendet bereits vorhandene Dienste und Clients. Der Adapter richtet
  keinen RDP-Dienst ein und ändert weder NLA, TLS noch Firewall. Die konkreten
  Host-/Clientbelege stehen im [RDP-Adaptervertrag](../../native/src/rdp/README.md).
- Win32-/X11-Familien verwenden belegte Mitgliedsquellen. Wayland-Portal und
  Android-Systemauswahl verleihen keine beliebige ferne Appauswahl, keine
  zusätzlichen Fokus-/Resize-/Close-Rechte und keine Appisolation.
- Die gewöhnliche Bildschirmsteuerung bleibt an ihre vorhandenen Rechte
  gebunden. Loginrechte und administratives SAS sind davon getrennt.

Statische Befunde und Beweisgrenzen stehen in
[Review B7](../plaene/2026-10-24-batches/review-b7.md) und
[Review B8](../plaene/2026-10-24-batches/review-b8.md).
Offene Produktarbeit wird ausschließlich in [TODO](../TODO.md) geführt.
