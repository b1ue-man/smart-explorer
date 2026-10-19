# Stand Design-Agent

Stand: 2026-10-19. Ergebnisdatei: `festlegungen.md` (1743 Zeilen). Jeder Abschnitt endet mit `Status: festgelegt`. Es wurde keine andere Datei als `festlegungen.md` und diese Datei geändert; kein Build, kein Test, kein Commit.

| Nr | Paket | Meilensteine | Abschnitt | Inhalt |
|---|---|---|---|---|
| 1 | U17 | DRV-M5, DRV-M6 | fertig | 33 Festlegungen (U17-1 bis U17-33) |
| 2 | U16 | SCR-M4 | fertig | 21 Festlegungen (U16-1 bis U16-21) |
| 3 | U19 | SCR-M5 | fertig | 13 Festlegungen (U19-1 bis U19-13) |
| 4 | U26 | VLT-M8, VLT-M9 | fertig | 15 Festlegungen (U26-1 bis U26-15) und Registrierung U26-30 |
| 5 | U37 | DSK-M7 | fertig | 20 Festlegungen (U37-1 bis U37-20) |
| 6 | U34 | DSK-M6, DSK-M8 | fertig | 22 Festlegungen (U34-1 bis U34-22) |
| 7 | U25 | SCR-M7 | fertig | 15 Festlegungen (U25-1 bis U25-15) |
| 8 | U29 | VLT-M10 | fertig | 13 Festlegungen (U29-1 bis U29-13) |
| 9 | U28 | SCR-M9 | fertig | 12 Festlegungen (U28-1 bis U28-12) |
| 10 | U35 | DSK-M9, DSK-M10 | fertig | 36 Festlegungen (U35-1 bis U35-36) |
| 11 | U38 | DSK-M11, DSK-M12 ohne die zwei Gesten | fertig | 18 Festlegungen (U38-1 bis U38-18) |
| 12 | U40 | DSK-M13, Gesten, `performGlobal` | fertig | 22 Festlegungen (U40-1 bis U40-22) |
| 13 | U22 | nur die Frage | fertig | 7 Fragen F1 bis F7 an den Sicherheitsagenten |
| 14 | U31 | nur die Bildschirme | fertig | 17 Festlegungen (U31-1 bis U31-17) und 8 Fragen S1 bis S8 |

Nutzerentscheidungen: keine.

Neue Annahmen dieser Festlegungen (zur Aufnahme in die Annahmenliste): A46 (Paketname der Auswahlaktivität), A47 (Systemfarben in Widgets), A48 (`PinItemRequest.accept(Bundle)` mit Widget-ID), A49 (`getPrivateSpaceSettingsIntent()` liefert `IntentSender?`), A50 (Messenger-Nachricht unter der Binder-Grenze).

Hinweise an den Hauptagenten (keine Nutzerentscheidung):
- Bestehende Grenzen ohne belegte technische Ursache: `MAX_SESSIONS` 8 und `MAX_DEVICE_SESSIONS` 4 (U10), `MAX_SUFFIX` 999 in `mount_point.rs`, `MAX_APPS` 256 (U27).
- Abweichungen vom Plan: Bildvergleich auf dem Desktop entfällt (TODO `VAULT-COMPARE-IMAGE`), `gpu_convert.rs` entfällt, keine neuen IPC-Varianten für Fensterliste und Standbild, DAMAGE entfällt, `setColorResources` wird nicht benutzt (P13), zwanzigste Bridge-Methode `vault.compareItems`, kein „zuletzt gesehen“ auf Android (TODO `LAUNCHER-LAST-SEEN`), kein Ziehen aus der Widget-Auswahl (TODO `LAUNCHER-WIDGET-DRAG`).
- U22: Die Eingabe im Hostprozess (`worker_wire::INPUT`) ist im Arbeitsbaum schon gebaut; Linux-Wayland (`portal_input.rs`) und der Zweig `host_backend.rs` sind nicht committet.
