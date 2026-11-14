# Maschinenanmeldung, TCP, RDP, Programme und Fensterfamilien

Quellstand: 2026-10-28, Kandidat aus [Verbinden ist die
Einrichtung](../plaene/2026-10-28-verbinden-ist-einrichtung/entwurf-einrichtung.md)
(r3–r5, r7) auf Basis von 0.5.188. Die folgenden Anschlüsse liegen im Quelltext
vor. Die gemeinsame Remote-Suite und Veröffentlichung stehen aus. `N/` bedeutet
`native/crates/`, `A/` bedeutet `android/app/src/main/java/app/smartexplorer/android/`.

## Erreichbare Oberflächen

| Fähigkeit | CLI und MCP | Desktop | Android und Bridge | Gemeinsame Grenze |
|---|---|---|---|---|
| Maschinenanmeldung | `se login status/provision/provision-network/target/open/sessions/session/close` (`N/cli/src/cli/login.rs:10`); MCP `login` (`N/cli/src/cli/mcp/catalog.rs:203`) | Geräteseite → Maschinenanmeldung (`N/gui/src/app/core/share_login_ui.rs:13`), eigener bildgebundener Renderer `login_viewer.rs` | `LoginTargetDialog` und `LoginViewScreen` (`A/ui/share/ShareDialogHost.kt:107`); `login.viewOpen`, Status, Fokus, Presented, Input, SAS, Release, Close (`N/mobile/src/mobile/os/shared/domains/login_view.rs:26`) | Gekoppeltes Maschinenziel über die gepinnte Share-Sitzung (`login_target_v1`); der Grant gilt nur für das gewährende Konto (SID/UID). Das Bildschirmrecht „Bildschirm steuern: Erlauben“ umfasst die Maschinenanmeldung (E21); ein eigenes Login-Recht gibt es nicht. Die Verdrahtung der Einrichtung und der Zielübertragung ist offen (TODO SELF-SETUP). |
| Lokale Dienste und TCP | `se tcp services/start/list/status/close` (`N/cli/src/cli/tcp.rs`); MCP `tcp_forward` mit `services` (`N/cli/src/cli/mcp/tcp_tool.rs`) | Geräteseite → „Dienste und TCP“ (`N/gui/src/app/core/share_tcp_ui.rs`): Dienste von selbst mit Adresse und „Öffnen“/„Adresse kopieren“; darunter „Weitere Weiterleitung“; se-Desktop-Kachel: Untermenü „Dienste“ | `TcpDialog`, `TcpApi`; `tcp.services`, `tcp.openService`, `tcp.*` (`N/mobile/src/mobile/os/shared/domains/tcp.rs`, `tcp_services.rs`) | Unter „Bildschirm steuern: Erlauben“ sind lokal lauschende Dienste freigegeben (nur Loopback: jedes Konto; Wildcard: nur Benutzerkonten; nie se-eigene Ports). Desktop bildet sie selbst auf `Geräteadresse:Port` ab (nur Programme desselben Kontos); über dem automatischen Budget bietet der Desktop „Öffnen“ und „Schließen“ je Dienst an (E50); Android öffnet je Dienst auf Tippen. Weitere Ziele brauchen das Recht „TCP-Weiterleitung“. Keine Bestätigung, keine feste Frist. |
| Remote Desktop | `se rdp open <gerät> [--remote-app <alias>]`, `local-probe/probe/clients/list/status/close` (`N/cli/src/cli/rdp.rs`); MCP `rdp` (`N/cli/src/cli/mcp/rdp_tool.rs`) | Geräteseite → „Remote Desktop“: ein Knopf plus je RemoteApp ein Knopf (`N/gui/src/app/core/share_rdp_ui.rs`); se-Desktop-Kachelmenü; „Dieses Gerät“ zeigt den eigenen Stand (`share_rdp_this_device.rs`) | `RdpDialog` zeigt den Grund des fehlenden Android-RDP-Programms und den Stand des Geräts; `rdp.open` braucht nur `deviceId`/`publicKey` | „Bildschirm steuern: Erlauben“ genügt. se richtet den Host selbst ein, wählt Beziehung, Dienst, Client, Vertrauen; die Sitzung lebt, solange der Client läuft. Nur die authentifizierte Probe ist Hostbeleg. |
| Installierte Anwendung starten | `se apps`, MCP `apps` (`N/cli/src/cli/mcp/catalog.rs:201`) | Gerätekatalog lädt von selbst (`N/gui/src/app/core/share_apps_ui.rs`); se-Desktop „Programme auf diesem Gerät …“ | `AppsDialog`, `apps.*` (`N/mobile/src/mobile/os/shared/domains/apps.rs`) | Katalogkennung, angemeldeter Hostnutzer und Sitzung. Unklarer Ausgang wird von se mit demselben Beleg geklärt; „Starten“ wartet so lange. Keine freie Shellausführung aus dem Appkatalog. |
| Fenster anderer Geräte | `se screen families` aus dem Gerätefeed, sonst Einzelabruf (`N/cli/src/cli/screen/families.rs`); MCP `screen` Aktion `families` fragt das Gerät weiterhin einzeln, `screen_window` öffnet ein Fenster | Geräteseite → „Fenster“ ohne Abrufknopf (`N/gui/src/app/core/family_picker.rs`); se-Desktop-Kachel: Fenster im Tooltip und Untermenü „Fenster“ | `FamilyPicker` liest `screen.feed`, sonst einmal `screen.families` | Der Hintergrunddienst des Betrachters hält je anwesendem Gerät einen Gerätefeed; weder Host- noch Betrachteroberfläche muss laufen. Fenster öffnen mit Steuerrecht, wenn das Gerät steuern lässt, sonst ansehend. |

Die Desktop-Geräteseite verbindet dieselben Fähigkeiten für Kontakte, direkte
Grants und Raummitglieder (`share_device_page_ui.rs:200`, `:378`, `:457`).
CLI, MCP und Desktop verwenden die registrierten Daemon-IPC-Wege; die mobile
Bridge verwendet denselben Kern und dieselben typisierten Rechtegrenzen.
Registrierungen: `N/engine/src/daemon/mod.rs`, `N/cli/src/cli/mcp/server.rs:146` und
`N/mobile/src/mobile/os/shared/domains/mod.rs:133`.

Android führt `OpenApps`, `OpenScreen`, `ShowScreenGrid` und
`ShowShareRequests` auch aus den Einstellungsbereichen Darstellung,
Fehlerprotokoll und Info zur Geräteansicht. Die Einstellungsnavigation erhält
die Anfrage bis zur Bearbeitung durch `ShareScreen`; Geräteidentität und
Programmziel bleiben erhalten (`A/ui/settings/SettingsNavigation.kt`,
`SettingsScreen.kt`, `A/ui/AppRoot.kt`). Der getrennte Desktopflächenprozess
registriert den Familienbetrachter selbst. Die eframe-Fläche und die
Wayland-Layer-Fläche zeichnen Programmauswahl und Fehler beim Öffnen einer
Fensterfamilie über dieselbe Funktion `show_surface`
(`N/gui/src/app/core/desktop_surface_app.rs`, `desktop_surface_grid_ui.rs`).

Zugeordnete X11-Combo-Popups und verschachtelte Menüs bestätigen den Fokus
ihres nächsten regulären Eigentümerfensters. Eingabeziel bleibt das tatsächlich
dargestellte Familienmitglied mit seiner unveränderten Identität und geprüften
Geometrie. Ein modaler Dialog bildet eine eigene Fokusgrenze; fremde,
abgelaufene oder nicht zugeordnete Fenster bestätigen den Fokus nicht.
Der letzte X11-Eingabeanschluss prüft die gebundene Geometrie erneut
(`N/engine/src/remote_window/os/x11/focus.rs`, `N/engine/src/screen/core/family_viewer.rs`,
`N/engine/src/screen/os/linux_os/host_backend.rs`).

## Verbinden ist die Einrichtung (ab 0.5.189)

Nutzerregeln 2026-10-28: „die eine freigabe muss reichen, der rest muss
laufen“, „er verbindet zwei geräte … tcp, alles“, „wenn n fenster offen ist
sehen es andere se desktops … per default“, „keinen random ass default
timeouts“. Nach Kopplung und „Bildschirm steuern: Erlauben“ tut die Person:

| Oberfläche | Vorher (0.5.188) | Jetzt |
|---|---|---|
| Desktop-Geräteseite, Remote Desktop | Beziehung wählen, „RDP-Fähigkeiten abfragen“, Dienst, Client, Darstellung, Lebensdauer, drei Kästchen, „Remote Desktop öffnen“, Fingerprint im mstsc-Dialog vergleichen (mindestens 7 Bedienschritte) | „Remote Desktop“ (1 Klick); Benutzer und Kennwort einmal im RDP-Programm (Linux ohne `sdl-freerdp3`: in der Anmeldemaske der Geräteseite, nur über die Standardeingabe des Programms weitergereicht) |
| Desktop-Geräteseite, Fenster | „Fensterliste abrufen / aktualisieren“, Kästchen „mit Steuerrecht“, „Öffnen“ (2–3 Klicks) | Liste steht da; „Steuern“/„Ansehen“ (1 Klick) |
| Desktop-Geräteseite, Dienste | Zielhost, Zielport, lokaler Port, Lebensdauer, Kästchen, „Starten“, Hostfreigabe je Port | Dienste stehen mit Adresse da; „Öffnen“ bzw. „Adresse kopieren“ (1 Klick) oder direkt `http://127.x.y.z:Port` im eigenen Programm (0 Klicks); „Öffnen“ und „Schließen“ auch für Dienste über dem Budget (1 Klick, E50) |
| Desktop-Geräteseite, Programme | Beziehung wählen, „Programmkatalog abfragen“, „Starten“; bei unklarem Ergebnis „Ergebnis abfragen“, Kästchen „neuen Auftrag“; „Bestätigtes Appfenster öffnen“ | „Starten“ (1 Klick); Klärung, Neustart nach Entsperren und Fensteröffnung macht se |
| se Desktop | Kachelmenü ohne Remote Desktop, keine Fenster, keine Dienste | Kachelmenü „Remote Desktop“, RemoteApps, „Fenster (n)“, „Dienste (n)“ (1 Klick); Tooltip listet Fenster und Dienste |
| Android | RDP-Abfrage-Knopf ohne Startmöglichkeit; TCP mit Lebensdauer und Kästchen; Fenster mit Aktualisieren und Steuerkästchen; Programme mit Kästchen und Abfrageknopf | RDP-Dialog nennt nur Grund und Stand; Dienste „Öffnen“/„Schließen“ (1 Tippen); Fensterliste aus dem Feed (1 Tippen); Programme klären sich selbst |
| CLI | `se rdp open <gerät> --service … --client … --acknowledge-local-access --acknowledge-session-effect [--acknowledge-certificate-dialog]`; `se tcp start … --acknowledge-local-access` | `se rdp open <gerät>` (`--service`, `--client`, `--wait` übersteuern weiterhin, verborgen); `se tcp services <gerät>`; `se tcp start <gerät> --target-port P`; `se tcp open <gerät> <port>` und `se tcp close-service <gerät> <port>` für Dienste über dem Budget (E50) |
| MCP | `rdp open` mit `service`, `client`, Bestätigungen; `tcp_forward start` mit `acknowledge_local_access` | `rdp open` mit nur `device`; `tcp_forward services`; `start` ohne Bestätigung; `tcp_forward` mit `open_service` und `close_service` (E50) |

Zustandstexte, die jede Oberfläche gleich zeigt: „Remote Desktop wird geöffnet
… fragt es dort einmal nach der Administratorfreigabe“ (Einrichtung läuft),
„Administratorfreigabe für Remote Desktop wurde abgelehnt …“ (Freigabe nötig),
„Windows Home kann keinen Remote-Desktop-Zugang anbieten …“ und die übrigen
`impossible:`-Gründe (nicht möglich), „Gerät ist nicht erreichbar“,
„Gegenstelle älter als 0.5.189 …“, „Startergebnis wird geklärt“ und
„Hostsitzung gesperrt – zum Entsperren den Bildschirm öffnen“. Die Texte des
Einrichtungsstands stammen aus `SetupState::text` (`N/engine/src/rdp/core/types.rs`), die
Projektion aus `N/gui/src/desktop/core/tile_feed.rs`.

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
Quellen: `N/engine/src/login/core/viewer_guard.rs`, `N/engine/src/login/os/shared/{helper,broker,viewer}.rs`.

Der native RDP-Besitzer hält Clientprozess und Tunnel gemeinsam; der Tunnel
hat keine feste Frist und endet mit dem Client, der Verbindung oder dem Entzug. Beenden,
Rechteentzug und Appupdate schließen beide; ein fehlgeschlagener Abschluss
bleibt sichtbar und wiederholbar. CLI-/MCP-/Bridge-`list/status` zeigen den
gemeinsamen Tunnelbestand und damit auch reine TCP-Tunnel. Die Desktopliste
zeigt die tatsächlich im Prozess besessenen RDP-Clients. Quellen:
`N/engine/src/rdp/os/shared/session.rs`, `N/gui/src/app/os/shared/rdp_exit_gate.rs` und
`N/cli/src/cli/mcp/rdp_shutdown.rs`.

## Plattformgrenzen und Administration

- Der privilegierte Login-Dienst richtet sich aus dem Bildschirmrecht „Bildschirm steuern: Erlauben“ selbst ein, aktualisiert sich je Version und wird nach dem letzten Entzug entfernt (E21). Die Auslösung durch das Bildschirmrecht ist im Arbeitsbaum verdrahtet (E42; Suite offen, TODO SELF-SETUP); der Skriptweg bleibt als Alternative. Details: [RELEASING](../RELEASING.md#optional-login-services).
- Android ist Loginbetrachter. Der Quellstand bietet dort keinen privilegierten
  Loginhost und keinen belegten nativen RDP-Clientadapter an; die Oberfläche
  zeigt den konkreten Fähigkeitsgrund. Es entsteht kein Ersatzstart über eine URI.
  Android kann nicht ermitteln, welche App eine lokale Verbindung öffnet; ein
  freigegebener Dienst bekommt dort erst auf „Öffnen“ einen lokalen Port, den
  jede App des Telefons erreicht, bis „Schließen“.
- RDP-Hosts richtet se selbst ein (Windows: UAC einmal je Version und Auslöser,
  keine Firewallregel; Linux: GNOME-Desktopfreigabe, KRdp oder xrdp, Paketinstallation
  über Root-Dienst oder polkit). Stand 2026-11-07: Die GNOME-Desktopfreigabe ist im lokalen
  Netz erreichbar, solange ein Gerät „Bildschirm steuern“ darf; die Probe meldet das
  (`evidence`, `diagnostics`). xrdp lauscht nur auf Loopback. se startet KRdp mit `--address
  127.0.0.1` (E29); eine laufende KRdp-Einheit ohne diese Adresse meldet die Probe als Beleg (E56). Bestehende
  NLA-, TLS-, Firewall- und netzweite xrdp-Einstellungen ohne se-Marker
  bleiben. Die Host-/Clientbelege stehen im
  [RDP-Adaptervertrag](../../native/crates/engine/src/rdp/README.md).
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

## Stand 2026-11-07: Maschinenanmeldung, Dienste, Räume

- **Maschinenanmeldung:** Das Gateway nutzt nur die Relays des im Programm konfigurierten
  Share-Servers (mit dessen Pins und Klartext-Regel). Ohne konfigurierten Server bleiben nur
  Direktwege. Eine bestehende Einrichtung mit gespeichertem `use_default_relays: true` erhält die Relays des konfigurierten Share-Servers über die Netzdatei, die der Dienst bei einem Steuerrecht selbst schreibt (E21). Die Einspeisung der steuernden Geräte ist im Arbeitsbaum verdrahtet (E42; Suite offen, TODO SEC-UPDATE-GATEWAY-RELAYS); `se login provision-network VERZEICHNIS` (Root bzw. Administrator; schreibt nur `gateway-network.json`) bleibt der Alternativweg. Details: [RELEASING](../RELEASING.md#optional-login-services). Gegenstellen ohne
  Freigabe erhalten `permission_missing` aus einem Budget von 8 und werden nach 1 s getrennt; ist
  das Budget voll, werden Verbindungen ohne Hello getrennt.
- **Maschinenanmeldung ohne Skript (E42 bis E44, Stand 2026-11-14, im Arbeitsbaum):** Ein Gerät mit
  „Bildschirm steuern: Erlauben“ richtet den Login-Dienst selbst ein (E42). Das Ziel der
  Maschinenanmeldung kommt über die gekoppelte Share-Sitzung (E43, Merkmal `login_target_v1`);
  für ein Gerät ohne dieses Merkmal vermerkt der Betrachter `login_target_legacy_peer`. Eine
  Betriebssystem-Freigabe für den RDP- und den Login-Schritt fragt einmal (E44). Die Wege
  `se login provision`, `se login provision-network` und `se login open TARGET_FILE` bleiben als
  Alternativen erhalten. Offen (TODO SELF-SETUP): Abruf des Ziels nach Verengung des Rechts,
  Löschen des Ziels beim Entfernen eines Kontakts, Suite.
- **Automatisch gebundene Dienste und TCP:** Der Desktop bindet die Dienste aller Geräte bis zu
  einem Budget, das aus dem Betriebssystem folgt. Linux: Der Desktop hebt beim ersten Anwenden
  einer Dienstliste das weiche Limit `RLIMIT_NOFILE` auf das harte Limit an (höchstens
  `fs.nr_open`) und erlaubt ein Viertel des danach geltenden weichen Limits, mindestens 256
  Dienste aller Geräte gleichzeitig. Windows: keine globale Grenze. Android: Die Automatik bindet
  nichts; ein Dienst öffnet sich beim Antippen, und das Budget wird dort nicht gelesen. Ein Gerät
  liefert höchstens 512 Dienste (`MAX_SERVICES`). Erreicht der Desktop unter Linux das Budget `N`,
  zeigt der Dienst ohne Adresse den Text „Nicht automatisch verbunden: höchstens N Dienste aller
  Geräte gleichzeitig“; die Schaltflächen „Öffnen“ und „Schließen“ auf dem Desktop öffnen und
  schließen einen solchen Dienst über das Budget hinaus (E50, im Arbeitsbaum; Suite offen, siehe
  [TODO](../TODO.md), SEC-DESKTOP-OEFFNEN). Ein geschlossener Dienst bleibt bei weiteren
  Feed-Updates zu, bis „Öffnen“; „Öffnen“ gilt nur für einen Port, den der aktuelle Feed nennt.
  „Schließen“ eines nicht freigegebenen Ports meldet auf CLI, Desktop und Android denselben Text
  „Port N ist auf dem Gerät nicht freigegeben“.
  Eingehende und eigene Tunnel haben je 64 Plätze. Ein Host prüft vor der
  Namensauflösung und der Diensteliste, ob eine Regel oder das Steuerrecht das Ziel nennt.
  Peers ohne Regel erhalten `Refused` mit dem bisherigen Text; bei erschöpftem Zielvorrat
  erhalten sie `TCP tunnel admission exhausted`.
- **Tunnel als interaktive Sitzung und TCP-Pumpe (Quellstand 2026-11-14, Plan
  `docs/plaene/2026-11-14-verbindungen-je-zweck/plan.md` E3, E7, P3; nicht gebaut, nicht
  veröffentlicht):** Jeder laufende Tunnel zählt im Präsenzregister als interaktiv
  (`share/os/shared/tcp_pump.rs`, Art `TcpPump`), also jeder Tunnel von „Bildschirm steuern: Erlauben“,
  vom Fernzugriff und von RDP, auf der Seite, die den Tunnel pumpt (Gerät mit lokalem Socket und Gastgeber
  des Dienstes). Solange einer läuft, ziehen sich alle Dateiübertragungen dieses Geräts nach der
  Warteschlangenverzögerung zurück und öffnen keine weitere Zusatzverbindung
  (`docs/TRANSFER_ENGINE.md`, Abschnitt vom 2026-11-14). Die Pumpe hat ein Routenabo je Tunnel (nicht je
  Lese- oder Schreibvorgang), fragt den Routenmonitor einmal je Sekunde und je Richtung, zählt die
  Leerlaufgrenze von 120 Sekunden nur in aktiver Zeit (eine Routenpause hält sie an) und kopiert mit 8 KiB
  je Richtung, die einmalig auf 64 KiB wachsen, wenn ein Lesevorgang den Puffer ganz füllt; alle Pumpen
  des Prozesses teilen sich ein Budget von 16 MiB für Puffer. Die Verbindung eines Tunnels hat das
  Sendebündel 40/80. Bedienung, Rechte, Plätze und Fehlertexte bleiben unverändert; die Endwerte des
  Puffers legt die Messung M4 fest (TODO `VERB-MESSUNG`). Das Verhalten der Pumpe ist auf allen
  Oberflächen gleich, weil sie keine Einstellung hat: CLI (`se tcp`, `se rdp`), Desktop, Android und MCP
  (`tcp_forward`) bedienen dieselbe Pumpe.
- **Räume:** Höchstens 1.024 zugelassene Mitglieder je Raum. Erreicht ein Raum diese Zahl, verschwindet
  das Mitglied mit dem ältesten letzten Kontakt, das keinen eigenen Zustand trägt (keine Freigabe,
  keine Sperre, keine Warteanfrage). Es erscheint mit seiner nächsten gültigen Presence wieder.
