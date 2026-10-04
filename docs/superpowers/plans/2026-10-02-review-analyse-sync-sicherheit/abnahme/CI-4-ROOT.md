# RV1 – Roots begrenzte vierte Diagnosekorrektur

Stand: 2026-10-04. Evidenz ist derselbe beendete Run
[37162485159](https://github.com/b1ue-man/smart-explorer/actions/runs/37162485159)
auf `ac3b0c9098963fae386e94558f2f3ca1bb240740`; kein neuer Projekt-Review.

Der kandidatengebundene Remote-Formatpatch mit SHA-256
`74bb3290128ac40f9f8bd17d36043972d2d54cc238866d2c48dc7a95815d818c`
wurde nach sicherer Pfadprüfung bytegleich übernommen und als `74a2b67d`
committed. Kein lokaler Formatter wurde ausgeführt.

Die Room-Fixture vergleicht jetzt den vollständigen gepinnten Share-Server
exakt. Ein leerer Rootpfad wird über Android `Uri` zu `/` normalisiert.
Die frisch gelesenen Primärverträge stehen vor dem Edit in
[android-share-uri.md](../../../../refs/android-share-uri.md).
Schema, Host/Port, nichtleerer Pfad, Query und Pinfragment bleiben erhalten.
Alle bisherigen Verbindungs-, Join-, Member-, Discovery- und
Download-/SHA-256-Assertions bleiben unverändert.

Statischer Self-Review: API-Signaturen und Attribute mit der lokalen Ref
abgeglichen; der Diff enthält ausschließlich Import, privaten
Darstellungshelfer und die betroffene Vergleichsassertion.
Keine lokalen Builds, Compiler, Tests oder Geräteoperationen.
Die funktionale Bestätigung gehört weiterhin ausschließlich zur selben
integrierten Remote-RV1-Suite. Keine offene APIabhängigkeit in diesem Block.

Gelesen: eigener Abschnitt `ci-fourth-fixes.md`, `ShareRoomTaskTest.kt`,
`docs/refs/INDEX.md`, relevante lokale TLS-Ref-Navigation, Android
`Uri`/`Uri.Builder` und RFC 6455 Abschnitt 3 sowie eigene gesicherte Ref.
Geändert: `ShareRoomTaskTest.kt`, `docs/refs/INDEX.md`, eigener
Planabschnitt. Erstellt: `docs/refs/android-share-uri.md`, dieser Handoff.
Entscheidung: ausschließlich leeren Rootpfad normalisieren; kein
Substringvergleich und keine Entfernung des Pins.
