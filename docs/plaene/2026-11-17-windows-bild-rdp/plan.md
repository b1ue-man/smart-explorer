# Windows-Bild, RDP-Konto, virtueller Bildschirm (Auftrag 2026-11-17)

Stand: 2026-11-17. Ein Batch, ein Release am Ende (Nutzer 2026-11-14: „ein release wenn du fertig bist“).

## Ziel und Nutzeraussagen

| Kennung | Nutzeraussage (wörtlich) | TODO |
|---|---|---|
| A1 | „windows zu windows bildschirm anschauen und steuern klappt nicht … beim remote kommt das popup von se dass der bildschirm angesehen wird, aber bei dem der ihn remote sehen odersteuern will, schwarz“; „kein zusatzsetup über die freigaben durch den nutzer“ | `VERB-WIN-SCHWARZ`, `VERB-WIN-KODIERER` |
| A2 | „ich will dass es beim anzeigen und steuern eines bildschirms kein popup auf b gibt“; „das muss weg. komplett und ersatzlos“ | erledigt in `23a3270f` |
| A3 | „bei der nutzereingabe nimmt es dann standardmäsig die domain von maschiene a. nicht b“; „automatisch wenn möglich ja klar elevation wenn nötig aber so sachen wie teamviewer bekommen das auch hin“ | `RDP-ZIELKONTO` |
| A4 | Frage, ob se einen eigenen Bildschirm simuliert und die Auflösung für reine Remote-Server einstellbar ist; Antwort auf das Angebot: „ja“ | `VIRTUELLER-BILDSCHIRM` |
| A5 | Beifund: große Dateien von einem Gerät mit langsamem Upload brechen ab | `SHARE-ANTWORT-FRIST`, erledigt in `66aa8967` |
| A6 | Beifund: „Maschinenanmeldung: fehlgeschlagen“ auf dem Linux-Server | `LOGIN-SELBSTEINRICHTUNG-LINUX` |

## Belege

Messung 2026-11-17 mit `se screen bench` zwischen zwei Windows-Geräten (Testbild, v0.5.197):

| Gastgeber → Betrachter | Codec | Ergebnis |
|---|---|---|
| NVIDIA RTX A4500 (VM) → AMD Radeon 890M | Kacheln | 155 Bilder/s, 0 kaputt |
| dto. | H.264 („NVIDIA H.264 Encoder MFT“) | 4,6 Bilder/s gemessen, Kodieren p50 1064 ms, `fps_limit` `encode_time` |
| dto. | HEVC | kein intaktes Bild |
| AMD Radeon 890M → NVIDIA (VM) | Kacheln | 141 Bilder/s |
| dto. | H.264, HEVC | kein intaktes Bild |

Der direkte NVENC-Weg stand im Register an erster Stelle, gewählt wurde der Media-Foundation-Transform (Ursache unbekannt).

## Stand der Pakete

- P1 erledigt (`76367d2d`): DXGI erstes Bild, Codec-Wächter des Betrachters, MF-Hardware-Dekodierer an Software, Kodierer-Rücksetzung.
- P2 erledigt (`23a3270f`): kein Fenster auf dem gesehenen Gerät.
- P3 offen: Kodierer weisen vor ihrem Angebot zur Laufzeit nach, dass sie ein Testbild intakt und schnell genug liefern; Ursachen der drei gemessenen Fehler.
- P4 offen: RDP mit dem Konto von B, ohne Sicherheitswarnung, Anmeldung ohne oder mit höchstens einmaliger Kennworteingabe (Recherche).
- P5 offen: virtueller Bildschirm in der Auflösung des Betrachters (Linux, Windows; Recherche).
- P6 offen: Maschinenanmeldung der Selbsteinrichtung unter Linux.
- Danach: eine Suite, ein Release, lokale Installationen.

## Entscheidungen zum virtuellen Bildschirm unter Windows (Nutzer 2026-11-17)

- Weg: den MIT-lizenzierten „Virtual Display Driver“ (VirtualDrivers) mitliefern; ein eigenes Code-Signatur-Zertifikat ist keine Option (Nutzer: „b, a ist keine option. ich habe kein problem beim update iw auf ok zu drücken, nur beim nutzen dann noch irgend ein setup machen zu müssen“).
- Befund 2026-11-17: Release 24.10.27 (`VirtualDisplayDriver-x64.zip`) trägt im Katalog `mttvdd.cat` nur ein selbst ausgestelltes Zertifikat „CN = Virtual Display Driver“; dessen `installCert.bat` trägt es als vertrauenswürdig ein. Dieses fremde Zertifikat wird nicht übernommen.
- Signatur: im erhöhten Schritt des Updates erzeugt se auf dem Gerät ein eigenes Code-Signatur-Zertifikat, signiert den Katalog, trägt das Zertifikat in `LocalMachine\Root` und `LocalMachine\TrustedPublisher` ein und löscht den privaten Schlüssel sofort.
- Nur bei Änderung (Nutzer: „wenn ein update nichts am treiber ändert möchte ich nicht neu signieren müssen“): signiert und installiert wird nur, wenn der mitgelieferte Treiber (SHA-256 des Katalogs und der Treiberdateien) vom installierten abweicht, der Treiber fehlt oder nicht funktioniert.
- Nutzung: ohne physischen Monitor legt se den virtuellen Monitor in der Auflösung des Betrachters ohne Abfrage an.
- Ungeprüft bis zum Gerätelauf: Annahme eines lokal signierten Benutzermodus-Treibers bei Secure Boot; DXGI-Aufnahme eines IddCx-Monitors.
