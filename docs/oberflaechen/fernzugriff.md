# Maschinenanmeldung, TCP, RDP, Programme und Fensterfamilien

Quellstand: 2026-10-28, Kandidat aus [Verbinden ist die
Einrichtung](../plaene/2026-10-28-verbinden-ist-einrichtung/entwurf-einrichtung.md)
(r3–r5, r7) auf Basis von 0.5.188. Die folgenden Anschlüsse liegen im Quelltext
vor. Die gemeinsame Remote-Suite und Veröffentlichung stehen aus. `N/` bedeutet
`native/src/`, `A/` bedeutet `android/app/src/main/java/app/smartexplorer/android/`.

## Erreichbare Oberflächen

| Fähigkeit | CLI und MCP | Desktop | Android und Bridge | Gemeinsame Grenze |
|---|---|---|---|---|
| Maschinenanmeldung | `se login status/provision/target/open/sessions/session/close` (`N/cli/login.rs:10`); MCP `login` (`N/cli/mcp/catalog.rs:203`) | Geräteseite → Maschinenanmeldung (`N/app/core/share_login_ui.rs:13`), eigener bildgebundener Renderer `login_viewer.rs` | `LoginTargetDialog` und `LoginViewScreen` (`A/ui/share/ShareDialogHost.kt:107`); `login.viewOpen`, Status, Fokus, Presented, Input, SAS, Release, Close (`N/mobile/os/shared/domains/login_view.rs:26`) | Explizites öffentliches Maschinenziel, bestätigter Maschinenpin, administrativer Grant und SID/UID. Ein gewöhnlicher Gerätegrant erteilt diese Rechte nicht. |
| Lokale Dienste und TCP | `se tcp services/start/list/status/close` (`N/cli/tcp.rs`); MCP `tcp_forward` mit `services` (`N/cli/mcp/tcp_tool.rs`) | Geräteseite → „Dienste und TCP“ (`N/app/core/share_tcp_ui.rs`): Dienste von selbst mit Adresse und „Öffnen“/„Adresse kopieren“; darunter „Weitere Weiterleitung“; se-Desktop-Kachel: Untermenü „Dienste“ | `TcpDialog`, `TcpApi`; `tcp.services`, `tcp.openService`, `tcp.*` (`N/mobile/os/shared/domains/tcp.rs`, `tcp_services.rs`) | Unter „Bildschirm steuern: Erlauben“ sind lokal lauschende Dienste freigegeben (nur Loopback: jedes Konto; Wildcard: nur Benutzerkonten; nie se-eigene Ports). Desktop bildet sie selbst auf `Geräteadresse:Port` ab (nur Programme desselben Kontos); Android öffnet je Dienst auf Tippen. Weitere Ziele brauchen das Recht „TCP-Weiterleitung“. Keine Bestätigung, keine feste Frist. |
| Remote Desktop | `se rdp open <gerät> [--remote-app <alias>]`, `local-probe/probe/clients/list/status/close` (`N/cli/rdp.rs`); MCP `rdp` (`N/cli/mcp/rdp_tool.rs`) | Geräteseite → „Remote Desktop“: ein Knopf plus je RemoteApp ein Knopf (`N/app/core/share_rdp_ui.rs`); se-Desktop-Kachelmenü; „Dieses Gerät“ zeigt den eigenen Stand (`share_rdp_this_device.rs`) | `RdpDialog` zeigt den Grund des fehlenden Android-RDP-Programms und den Stand des Geräts; `rdp.open` braucht nur `deviceId`/`publicKey` | „Bildschirm steuern: Erlauben“ genügt. se richtet den Host selbst ein, wählt Beziehung, Dienst, Client, Vertrauen; die Sitzung lebt, solange der Client läuft. Nur die authentifizierte Probe ist Hostbeleg. |
| Installierte Anwendung starten | `se apps`, MCP `apps` (`N/cli/mcp/catalog.rs:201`) | Gerätekatalog lädt von selbst (`N/app/core/share_apps_ui.rs`); se-Desktop „Programme auf diesem Gerät …“ | `AppsDialog`, `apps.*` (`N/mobile/os/shared/domains/apps.rs`) | Katalogkennung, angemeldeter Hostnutzer und Sitzung. Unklarer Ausgang wird von se mit demselben Beleg geklärt; „Starten“ wartet so lange. Keine freie Shellausführung aus dem Appkatalog. |
| Fenster anderer Geräte | `se screen families` aus dem Gerätefeed, sonst Einzelabruf (`N/cli/screen/families.rs`); MCP `screen` Aktion `families` fragt das Gerät weiterhin einzeln, `screen_window` öffnet ein Fenster | Geräteseite → „Fenster“ ohne Abrufknopf (`N/app/core/family_picker.rs`); se-Desktop-Kachel: Fenster im Tooltip und Untermenü „Fenster“ | `FamilyPicker` liest `screen.feed`, sonst einmal `screen.families` | Der Hintergrunddienst des Betrachters hält je anwesendem Gerät einen Gerätefeed; weder Host- noch Betrachteroberfläche muss laufen. Fenster öffnen mit Steuerrecht, wenn das Gerät steuern lässt, sonst ansehend. |

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

Zugeordnete X11-Combo-Popups und verschachtelte Menüs bestätigen den Fokus
ihres nächsten regulären Eigentümerfensters. Eingabeziel bleibt das tatsächlich
dargestellte Familienmitglied mit seiner unveränderten Identität und geprüften
Geometrie. Ein modaler Dialog bildet eine eigene Fokusgrenze; fremde,
abgelaufene oder nicht zugeordnete Fenster bestätigen den Fokus nicht.
Der letzte X11-Eingabeanschluss prüft die gebundene Geometrie erneut
(`N/remote_window/os/x11/focus.rs`, `N/screen/core/family_viewer.rs`,
`N/screen/os/linux_os/host_backend.rs`).

## Verbinden ist die Einrichtung (ab 0.5.189)

Nutzerregeln 2026-10-28: „die eine freigabe muss reichen, der rest muss
laufen“, „er verbindet zwei geräte … tcp, alles“, „wenn n fenster offen ist
sehen es andere se desktops … per default“, „keinen random ass default
timeouts“. Nach Kopplung und „Bildschirm steuern: Erlauben“ tut die Person:

| Oberfläche | Vorher (0.5.188) | Jetzt |
|---|---|---|
| Desktop-Geräteseite, Remote Desktop | Beziehung wählen, „RDP-Fähigkeiten abfragen“, Dienst, Client, Darstellung, Lebensdauer, drei Kästchen, „Remote Desktop öffnen“, Fingerprint im mstsc-Dialog vergleichen (mindestens 7 Bedienschritte) | „Remote Desktop“ (1 Klick); Benutzer und Kennwort einmal im RDP-Programm (Linux ohne `sdl-freerdp3`: in der Anmeldemaske der Geräteseite, nur über die Standardeingabe des Programms weitergereicht) |
| Desktop-Geräteseite, Fenster | „Fensterliste abrufen / aktualisieren“, Kästchen „mit Steuerrecht“, „Öffnen“ (2–3 Klicks) | Liste steht da; „Steuern“/„Ansehen“ (1 Klick) |
| Desktop-Geräteseite, Dienste | Zielhost, Zielport, lokaler Port, Lebensdauer, Kästchen, „Starten“, Hostfreigabe je Port | Dienste stehen mit Adresse da; „Öffnen“ bzw. „Adresse kopieren“ (1 Klick) oder direkt `http://127.x.y.z:Port` im eigenen Programm (0 Klicks) |
| Desktop-Geräteseite, Programme | Beziehung wählen, „Programmkatalog abfragen“, „Starten“; bei unklarem Ergebnis „Ergebnis abfragen“, Kästchen „neuen Auftrag“; „Bestätigtes Appfenster öffnen“ | „Starten“ (1 Klick); Klärung, Neustart nach Entsperren und Fensteröffnung macht se |
| se Desktop | Kachelmenü ohne Remote Desktop, keine Fenster, keine Dienste | Kachelmenü „Remote Desktop“, RemoteApps, „Fenster (n)“, „Dienste (n)“ (1 Klick); Tooltip listet Fenster und Dienste |
| Android | RDP-Abfrage-Knopf ohne Startmöglichkeit; TCP mit Lebensdauer und Kästchen; Fenster mit Aktualisieren und Steuerkästchen; Programme mit Kästchen und Abfrageknopf | RDP-Dialog nennt nur Grund und Stand; Dienste „Öffnen“/„Schließen“ (1 Tippen); Fensterliste aus dem Feed (1 Tippen); Programme klären sich selbst |
| CLI | `se rdp open <gerät> --service … --client … --acknowledge-local-access --acknowledge-session-effect [--acknowledge-certificate-dialog]`; `se tcp start … --acknowledge-local-access` | `se rdp open <gerät>` (`--service`, `--client`, `--wait` übersteuern weiterhin, verborgen); `se tcp services <gerät>`; `se tcp start <gerät> --target-port P` |
| MCP | `rdp open` mit `service`, `client`, Bestätigungen; `tcp_forward start` mit `acknowledge_local_access` | `rdp open` mit nur `device`; `tcp_forward services`; `start` ohne Bestätigung |

Zustandstexte, die jede Oberfläche gleich zeigt: „Remote Desktop wird geöffnet
… fragt es dort einmal nach der Administratorfreigabe“ (Einrichtung läuft),
„Administratorfreigabe für Remote Desktop wurde abgelehnt …“ (Freigabe nötig),
„Windows Home kann keinen Remote-Desktop-Zugang anbieten …“ und die übrigen
`impossible:`-Gründe (nicht möglich), „Gerät ist nicht erreichbar“,
„Gegenstelle älter als 0.5.189 …“, „Startergebnis wird geklärt“ und
„Hostsitzung gesperrt – zum Entsperren den Bildschirm öffnen“. Die Texte des
Einrichtungsstands stammen aus `SetupState::text` (`N/rdp/core/types.rs`), die
Projektion aus `N/desktop/core/tile_feed.rs`.

Zeitgrenzen: keine Bedienwartezeit und keine Lebensdauer als Vorgabe. Probe,
Start, Katalog, Belege und Symbolabruf warten, bis das Gerät antwortet, die
Person die Seite verlässt oder die Verbindung als tot erkannt wird (QUIC 20 s
Leerlauf). Eine ausdrücklich gewählte Frist (`--lifetime-secs`,
`lifetimeSecs`, MCP `lifetime_secs`, `wait`) bleibt möglich.

## Besitz, Darstellung und Beendigung

Loginbilder laufen binär zum Renderer. CLI/MCP erhalten Status und Bildmetadaten;
sie transportieren keine Loginpixel in JSON. Eingaben benötigen den Beleg des
tatsächlich dargestellten Bildes und die aktuelle Präsentationsepoche.
Ein erkannter Sitzungs-, Desktop- oder Geometriewechsel beendet den alten
Login-Kontext. Gehaltene Eingaben werden freigegeben. Wiederaufnahme benötigt
eine neue ausdrückliche Sitzung und einen neuen Darstellungsbeleg.
Quellen: `N/login/core/viewer_guard.rs`, `N/login/os/shared/{helper,broker,viewer}.rs`.

Der native RDP-Besitzer hält Clientprozess und Tunnel gemeinsam; der Tunnel
hat keine feste Frist und endet mit dem Client, der Verbindung oder dem Entzug. Beenden,
Rechteentzug und Appupdate schließen beide; ein fehlgeschlagener Abschluss
bleibt sichtbar und wiederholbar. CLI-/MCP-/Bridge-`list/status` zeigen den
gemeinsamen Tunnelbestand und damit auch reine TCP-Tunnel. Die Desktopliste
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
  Android kann nicht ermitteln, welche App eine lokale Verbindung öffnet; ein
  freigegebener Dienst bekommt dort erst auf „Öffnen“ einen lokalen Port, den
  jede App des Telefons erreicht, bis „Schließen“.
- RDP-Hosts richtet se selbst ein (Windows: UAC einmal je Version und Auslöser,
  keine Firewallregel; Linux: GNOME-Desktopfreigabe, KRdp oder xrdp nur auf
  Loopback, Paketinstallation über Root-Dienst oder polkit). Bestehende
  NLA-, TLS-, Firewall- und netzweite xrdp-Einstellungen ohne se-Marker
  bleiben. Die Host-/Clientbelege stehen im
  [RDP-Adaptervertrag](../../native/src/rdp/README.md).
- Wildcard-gebundene Systemdienste (RPC, SMB, Anmeldedienste) und se-eigene
  Ports werden nie automatisch freigegeben; das Netz des Hosts bleibt beim
  Recht „TCP-Weiterleitung“.
- Win32-/X11-Familien verwenden belegte Mitgliedsquellen. Wayland-Portal und
  Android-Systemauswahl verleihen keine beliebige ferne Appauswahl, keine
  zusätzlichen Fokus-/Resize-/Close-Rechte und keine Appisolation.
- Die gewöhnliche Bildschirmsteuerung bleibt an ihre vorhandenen Rechte
  gebunden. Loginrechte und administratives SAS sind davon getrennt.

Statische Befunde und Beweisgrenzen stehen in
[Review B7](../plaene/2026-10-24-batches/review-b7.md) und
[Review B8](../plaene/2026-10-24-batches/review-b8.md).
Offene Produktarbeit wird ausschließlich in [TODO](../TODO.md) geführt.
