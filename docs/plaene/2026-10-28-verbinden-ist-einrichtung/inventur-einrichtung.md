# Inventur: manuelle Schritte nach Kopplung und Bildschirmfreigabe

Stand: 2026-10-28, Quellstand `a67d7296` (v0.5.188 + Indikatorfix). Gelesen, nicht ausgeführt.
Ziel (Nutzer r3/r4/r7): Kopplung zweier Geräte plus die eine Bildschirmfreigabe ist die ganze
Einrichtung; keine weiteren Menüs, Optionen oder frei gewählten Zeitlimits.

## A. RDP, Desktop-GUI

| # | Fundstelle | Manueller Schritt | Ableitbar? |
|---|---|---|---|
| A1 | `app/core/share_rdp_ui.rs:159-164,171-179`; `rdp/core/admission.rs:35-37` | Klick „RDP-Fähigkeiten abfragen / aktualisieren“; Probe nach 5 min veraltet | ja: Probe beim Öffnen/Klick über den authentifizierten Kanal |
| A2 | `share_rdp_ui.rs:113-123` | Relation (Direkt/Raum) wählen | ja: anwesende Relation, Direkt vor Raum |
| A3 | `share_rdp_form.rs:29-40`; Vorgabe `share_rdp_ui.rs:233-238` | Dienst-Auswahl; Vorgabe erster Dienst, auch wenn nicht startbar; Linux bis 5 Dienste | teils: `HostService::startable()` (`rdp/core/types.rs:117-131`); Rangfolge mehrerer ist Gestaltungsentscheidung |
| A4 | `share_rdp_form.rs:84-106` | Client-Auswahl (erster vorausgewählt) | ja |
| A5 | `share_rdp_form.rs:118-142` | „Darstellung“: ganzer Desktop oder RemoteApp-Alias | echte Wahl, nur Windows Server |
| A6 | `share_rdp_form.rs:143-146`; `share_rdp_ui.rs:46`; `admission.rs:84-89`; `rdp/os/shared/session_owners.rs:215-232`; `session.rs:154-160` | Lebensdauer 1–86400 s, Vorgabe 900; bei Ablauf wird der Client beendet, keine Verlängerung | nein im Bestand; r7 verlangt Bindung an die Nutzung |
| A7 | `share_rdp_form.rs:157-161`; `admission.rs:78-79,134` | Kästchen „Erreichbarkeit durch jeden lokalen Prozess“; setzt sich bei jeder Änderung zurück | nur durch Entwurfsänderung (Listener nur für den gestarteten Client) |
| A8 | `share_rdp_form.rs:162-165`; `admission.rs:97-99`; `types.rs:30-43` | Kästchen Wirkung auf die Zielsitzung | Wirkung ist abgeleitet; Zustimmung liegt mit „Steuern: Erlauben“ beim Host bereits vor |
| A9 | `share_rdp_form.rs:166-170`; `admission.rs:108-112`; `rdp/os/windows/clients.rs:17` | nur mstsc: Kästchen und Fingerprintvergleich im Zertifikatdialog | im Bestand nein (`windows/profile.rs:69`) |
| A10 | `windows/clients.rs:29`; `linux_os/clients.rs:81-83`; `rdp/README.md` | Benutzername/Kennwort im nativen Client | Absicht: se reicht keine Zugangsdaten weiter |
| A11 | `share_rdp_form.rs:205-217` | Klick „Remote Desktop öffnen“ | die Handlung selbst |
| A12 | `share_rdp_ui.rs:224,283` | bis 30 s Warten auf Bestätigung/TCP-Freigabe | TCP zum eigenen RDP-Port ist unter Steuern erlaubt (siehe F) |

## B. RDP, CLI/MCP/Android-Brücke

| # | Fundstelle | Manueller Schritt |
|---|---|---|
| B1 | `cli/rdp.rs:32-37`; `cli/mcp/rdp_tool.rs:50-51`; `mobile/os/shared/domains/rdp.rs:141-142` | `--service`/`--client` Pflicht, vorher `se rdp probe`/`clients` |
| B2 | `cli/rdp.rs:38-39`; `rdp_tool.rs:39-48`; `mobile/.../rdp.rs:122`; `admission.rs:100-102` | Vorgabe `exact-peer-pin` wird immer abgelehnt; Policy muss zum Client passen |
| B3 | `cli/rdp.rs:43-48`; `rdp_tool.rs:17-18`; `mobile/.../rdp.rs:145-147` | drei `acknowledge_*`-Schalter |
| B4 | `cli/rdp.rs:49-52`; `rdp_tool.rs:61` | `--lifetime-secs` 900, `--wait` 30 |
| B5 | `linux_os/clients.rs:45,49,52` | FreeRDP gilt nur mit Terminal als stdin als verfügbar (GUI/MCP ohne Terminal: kein Client) |
| B6 | `rdp/os/mod.rs:8-16` | Android: kein RDP-Client, kein RDP-Host |

## C. Hostvoraussetzungen RDP (se richtet nichts ein, `rdp/README.md`)

| # | Fundstelle | Voraussetzung |
|---|---|---|
| C1 | `rdp/os/windows/probe.ps1:24`; `probe.rs:61-62` | Remotedesktop aktiv (`fDenyTSConnections=0`), „wird nicht automatisch aktiviert“ |
| C2 | `probe.ps1:9-11`; `probe.rs:50-60` | Edition Pro/Enterprise/Education/Server; Home kann nicht hosten |
| C3 | `probe.ps1:25-31` | TermService läuft und besitzt den Listener |
| C4 | `probe.ps1:34-35`; `types.rs:127-129` | Sicherheitsschicht TLS oder Negotiate |
| C5 | `probe.ps1:36-52`; `admission.rs:103-107` | eindeutig gebundenes lesbares Maschinenzertifikat |
| C6 | `probe.ps1:54-72` | RemoteApp nur Windows Server mit RDS-Rolle und Collection |
| C7 | `rdp/os/linux_os/gnome.rs` | grdctl, RDP aktiviert, Zertifikat lesbar; System-Remote-Login nie startbar (pkexec) |
| C8 | `linux_os/krdp.rs` | krdpserver, Benutzerunit, Zertifikat, bekannter Modus |
| C9 | `linux_os/xrdp.rs` | xrdp aktiv, `security_layer` tls/negotiate ausdrücklich, Zertifikat lesbar, genau ein Sitzungstyp |
| C10 | `app/core/share_apps_ui.rs:221-223` | angemeldete Sitzung für Programmstarts; Root-Dienst vor Anmeldung offen |

## D. Clientvoraussetzungen

| # | Fundstelle | Voraussetzung |
|---|---|---|
| D1 | `linux_os/clients.rs`; `client_trust.rs` | FreeRDP 3 aus root-eigenem Pfad, kein `LD_PRELOAD`, keine systemweite `certificates.json`, Display gesetzt |
| D2 | `windows/clients.rs:10-14` | `mstsc.exe` im Systemverzeichnis (Normalfall) |

## E. TCP-Weiterleitung

| # | Fundstelle | Manueller Schritt |
|---|---|---|
| E1 | `app/core/share_tcp_ui.rs:131-133,93-97,214-222` | Zielhost, Zielport, lokaler Port, Lebensdauer 900 |
| E2 | `share_tcp_ui.rs:137-138,252`; `cli/tcp.rs:26-28`; `cli/mcp/tcp_tool.rs:18,62`; `mobile/.../tcp.rs:157-158` | Bestätigung „jeder lokale Prozess erreicht den Loopbackport“ |
| E3 | `share_tcp_ui.rs:66-75,139`; `mobile/.../tcp.rs:79-88,128-132` | ausgehende gespeicherte Relation nötig; Host muss genau Relation/Richtung/Adresse/Port erlauben |
| E4 | `share_tcp_ui.rs:140,190`; `cli/tcp.rs:66-68` | bis 30 s Warten auf Hostbestätigung |

Gewährung: `share/core/access_steering.rs:43-48` — unter „Steuern: Erlauben“ nur Richtung
Peer→Host, Loopback, Port aus `OWN_RDP_PORTS` (gefüllt bei jeder RDP-Probe,
`share/core/node_apps_host.rs:19`); andere Ports/Richtungen brauchen eine Einzelfreigabe.

## F. Programme und Fenster

| # | Fundstelle | Manueller Schritt |
|---|---|---|
| F1 | `share_apps_ui.rs:157-161,325,328-335` | Klick „Programmkatalog abfragen / aktualisieren“ |
| F2 | `share_apps_ui.rs:90-102` | Relation wählen |
| F4 | `share_apps_ui.rs:202-210,269-279`; `share_apps_jobs.rs:158-180` | nach unklarem Ergebnis: Ergebnis manuell abfragen, Kästchen „neuen Auftrag erstellen“ |
| F5 | `share_apps_ui.rs:277,363` | „Lokale Zustimmung am Zielgerät erforderlich“ (unter Steuern laut `access_steering.rs:42` eingeschlossen) |
| F6 | `share_apps_ui.rs:184-198,369-398` | Klick „Bestätigtes Appfenster öffnen“ |
| F7 | `mobile/.../app_launch.rs:41-42`; `apps.rs:7-27` | Android-Host: lokale Zustimmung/Einzelfreigaben (Kotlin-UI ungelesen) |

Offen für den Entwurf: Fensterliste anderer Geräte ohne laufenden lokalen se Desktop (r5),
Ermittlung lauschender localhost-Dienste je Plattform (r5), Credentials im RDP-Client (A10).
