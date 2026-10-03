# V-LOCAL – Integration der vorhandenen Verbraucher

Stand: 2026-10-03. Implementiert, gemeinsame Remote-Abnahme ausstehend.
Die vollständigen Signaturen und Garantien stehen in
[anfragen/V-LOCAL.md](../anfragen/V-LOCAL.md).

- `DirectoryHandle` hält autorisierte Wurzel und Childgrenzen. Gewöhnliche Host-Aufträge
  verwenden `open_root`; lokale GUI-Lesekonsente verwenden `open_root_consented`.
- Metadaten, Listing und reguläre Childdateien sind handlegebunden. Childlinks und
  besondere Dateien bleiben Grenzen; Windows-Daten-Reparse behalten Datenverhalten.
- Private Ordner/Dateien erhalten ihre Rechte bei Erstellung. `secure_private_handle`
  und `DirectoryHandle::secure_private` dürfen nur eigene Altobjekte migrieren und
  prüfen das Ergebnis vor Datennutzung; S-LOCAL verbindet die Storage-Fassade.
- `QuarantinedChild` bindet das erfasste Objekt. Restore/Move ersetzen keine andere
  Datei, Fehler bewahren Inhalt. Die diagnostische Schreibweise autorisiert keinen
  frei pfadbasierten Shell- oder Löschaufruf.
- `ReadyPartial` hält Hybrid-Abfragen aktiv. Der FD-Watch-Anker gilt nur bei gehaltenem
  Clone; Windows meldet ohne sicheren Overlapped-Handoff keine vollständige Abdeckung.
- Der gemeinsame eigene Namensmatcher erkennt präzise
  `.se-private-<32 lowercase hex>.tmp` und die vorhandenen Quarantäne-/Stage-Muster.

Historische pfadbasierte Schreibmethoden ersetzen keine RootGuards. Consumer-Grenzen
für Host, UNC, Transfer und Apply sind im Anfragebericht einzeln zugeordnet; sie
werden in ihren vorhandenen Blöcken verbunden. Keine neue Wire-Version außer den
bereits vereinbarten additiven V1-/Agent-Änderungen.
