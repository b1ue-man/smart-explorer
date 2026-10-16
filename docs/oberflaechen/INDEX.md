# Oberflächen von Smart Explorer

Stand: 2026-10-16.

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

## Pflege

- Jede Änderung, die eine Fähigkeit, eine Einstellung, ein Datenfeld oder einen Zustand hinzufügt oder ändert, aktualisiert das zuständige Teildokument im selben Commit: Zeile der Fähigkeit, Beleg `Datei:Zeile` je Oberfläche, Lücke oder Ausnahme mit Grund.
- Vor einem Plan wird das Teildokument gegen den Code geprüft; die Task-Suite des Stapels deckt jede neu ergänzte Oberfläche ab.
- Ein neues Teildokument wird hier eingetragen.
