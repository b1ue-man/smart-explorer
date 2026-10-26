# Oberflächen von Smart Explorer

Stand: 2026-10-26; laufende Medien- und Fernzugriffsverträge ergänzt. Die Einträge beschreiben die jeweilige Oberfläche; veröffentlichte Versionen werden in den verlinkten Dokumenten getrennt belegt.

Zweck: Dieses Verzeichnis hält fest, welche Oberflächen das Projekt hat und über welche Oberfläche jede Fähigkeit und jede Einstellung erreichbar ist. Eine Fähigkeit ist erst fertig, wenn jede Oberfläche, die ihren Gegenstand zeigt oder steuert, sie gleichwertig zeigt und steuert. Ausnahmen (zum Beispiel die Terminal-Verbindung selbst, eine Plattform ohne die nötige Betriebssystemfunktion) stehen mit Grund im Teildokument.

## Oberflächen

| Oberfläche | Einstieg im Code | Art |
|---|---|---|
| CLI `se` | `native/src/bin/se.rs`, Befehle unter `native/src/cli/` | Terminal, Text- und JSON-Ausgabe, Shell-Vervollständigung (`native/src/cli/completions.rs`) |
| Desktop-GUI | `native/src/app/` (egui), Fenster und Seiten in `native/src/app/core/` | Windows und Linux |
| Android-App | `android/app/src/main/java/app/smartexplorer/android/` (Compose) | Bildschirme, Dialoge, Benachrichtigungen |
| Android-Bridge | `native/src/mobile/` (JSON-Methoden), Kotlin `android/.../api/` | Vertrag `docs/superpowers/plans/2026-09-25-android-apk/api.md` |
| Daemon-IPC | `native/src/daemon/os/shared/ipc_protocol.rs` | Loopback zwischen GUI/CLI und Hintergrund-Worker |
| MCP-Server `se mcp` | `native/src/cli/mcp/` (Katalog `catalog.rs`) | KI-Agenten; das Werkzeug `se` führt jeden CLI-Befehl aus |
| Netzprotokolle | Share/Exec `native/src/share/core/`, Agent `native/src/agent_proto/` | Gerät zu Gerät |
| Dokumentation | `README.md`, `docs/` | Bedienwege und Verträge |

## Teildokumente

| Bereich | Datei |
|---|---|
| Share (Geräte, Anfragen, Räume, Freigaben, Auffindbarkeit, Netzwerk) und Exec (Grants, Konto, Programmliste, Jobs, Protokoll) | [share-exec.md](share-exec.md) |
| Alle übrigen Einstellungen der App | [einstellungen.md](einstellungen.md) |
| Laufwerke (Einhängen unter Windows und Linux, Optionen, Gerätebaum „Verbundene Geräte als Ordner“, Tresor-Laufwerke, Android-Dokumentanbieter, Laufzeit) | [laufwerke.md](laufwerke.md) |
| Übertragungsentscheidungen, Wiederanlauf, optische Quellen und gezielter Teilscan | [uebertragungen.md](uebertragungen.md) |
| Tresore (Cryptomator-Tresore, Sperren, Tresor-Sync, Tresor-Laufwerke) | [tresore.md](tresore.md) |
| Bildschirm (Geräte, Sitzungen, Kodierung, Eingabe, Betrachter) | [bildschirm.md](bildschirm.md) |
| Maschinenanmeldung, TCP, RDP, Programmstart und Fensterfamilien | [fernzugriff.md](fernzugriff.md) |
| se Desktop und se Launcher | [desktop-launcher.md](desktop-launcher.md) |
| Eigene Android-Medienvorschau (Bilder, Video, Audio) | [medien.md](medien.md) |

## Kostenklassen

Die Teildokumente beschreiben den Aufwand einer Aufgabe in Handlungen. Die Klassen gelten für alle Oberflächen gleich:

| Klasse | Bedeutung |
|---|---|
| C0 | keine Handlung (Anzeige oder Standard ohne Zutun) |
| C1 | genau eine Handlung (ein Befehl, ein Klick, ein Tipp) an der Stelle, an der die Person schon ist |
| C2 | zwei oder drei Handlungen in einem Dialog oder auf einer Seite; Texteingabe nur als Auswahl aus einer Liste |
| C3 | Navigation zu einer anderen Seite plus Handlungen, zusammen vier bis sechs; oder Freitext je Eintrag (Programm- oder Kontoname) |
| C4 | Handlung oder Wissen außerhalb von se (Konto im Betriebssystem anlegen, Systemdatei bearbeiten, Administratorbestätigung) oder mehr als sechs Handlungen oder eine Eingabe je erlaubtem Programm |

Quelle der Definition: `docs/plaene/2026-10-17-rechte-desktop-tresore/strang-rechte.md`, Abschnitt C6 („Bedienwege“, Kostenklassen; Lücke B23).

## Pflege

- Jede Änderung, die eine Fähigkeit, eine Einstellung, ein Datenfeld oder einen Zustand hinzufügt oder ändert, aktualisiert das zuständige Teildokument im selben Commit: Zeile der Fähigkeit, Beleg `Datei:Zeile` je Oberfläche, Lücke oder Ausnahme mit Grund.
- Vor einem Plan wird das Teildokument gegen den Code geprüft; die Task-Suite des Stapels deckt jede neu ergänzte Oberfläche ab.
- Ein neues Teildokument wird hier eingetragen.
