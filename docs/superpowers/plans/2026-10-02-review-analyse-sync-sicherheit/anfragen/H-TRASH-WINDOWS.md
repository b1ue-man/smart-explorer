# H-TRASH-WINDOWS – Integrationsanfragen

Stand: 2026-10-03. Der eigene dokumentierte FA6/A34-Umsetzungsblock ist abgeschlossen.
Kein neuer Projekt-Review und keine angeforderte Scope-Erweiterung.

## An den Hauptagenten

1. **Integration/Status:** Eigene neue Module und additive Registrierungen aus
   [api-delta/H-TRASH-WINDOWS.md](../api-delta/H-TRASH-WINDOWS.md) integrieren.
   Die zuvor ausdrücklich gemeldete Windows-FA6-Unsupported-Grenze aus dem historischen
   H-ANALYSIS-Bericht ist durch den vollständigen Host-Papierkorb samt sichtbarem Restore-
   Consumer ersetzt. Globale Plan-/Integrationsstatusdateien bleiben ausschließlich
   Hauptagentenfläche; dieser Block ändert sie nicht.
2. **Eine Remote-Abnahme:** Die exakt aufgeführten `review_task_host_trash_*`-Signale
   aus dem API-Delta in die vorhandene einzige Task-Level-Suite aufnehmen. Den vollständigen
   Windows-Host-/Share-Recycle-Aufruf mit Länge/SHA, Capability-OS-Maske, Prozessneustart
   und sichtbarem Host-Share-Reiter im selben isolierten Hostbenutzer-/Datenprofil verbinden.
   Der Runner benötigt die bereits für V-LOCAL vorgesehenen Symlink-Rechte. Keine neue
   Suite, lokale Ausführung oder zusätzliche Abnahmematrix durch diesen Subagenten.
3. **Parent-Owned Abschluss:** Commit/Push, Root-Graph-Aktualisierung und anschließender
   Release bleiben beim Hauptagenten. Der Arbeitsbaum enthält bewusst die Quellen und
   vorbereiteten Signale ohne irgendeinen Lauf-/Compiler-/Power-loss-Nachweis.

## Aufgelöste Vertragsfragen

- **Private Intent-Dauerhaftigkeit:** Der Hauptagent hat bestätigt, dass Windows
  `creds::private_storage::sync_directory` absichtlich keine NTFS-Directory-Flush-Garantie
  liefert. Nach EXCL-ID-Reservierung verwendet `Store::persist` deshalb genau die
  bestehende `support_dirs::write_private_atomic`-Fassade (private Stage, file.sync_all,
  vorhandene V1-Write-through-Promotion). Kein neuer unbewiesener Storage-Vertrag.
- **Windows-Quarantäne:** V-LOCAL cd8632d ist Voraussetzung. Die einzige Ergänzung ist der
  typed/validated gewählte Slot. Vollständige FileID/Volume, confirmed DELETE-Guard,
  FILE_SHARE_READ, NoReplace und vorhandenes Restore bleiben unverändert.
- **Auffindbarkeit vor Capture/bei Restore:** Der immutable Record enthält vor dem ersten
  Rename die exakte UTF-16-Originalzuordnung und beide Held-Dateinamen. Restart-Katalog
  und UI finden beide Positionen; Konfliktfehler erhalten Inhalt und nennen die Record-ID.
- **Native Windows-Papierkorbgrenze:** Es wird ausdrücklich der sichtbare Smart-Explorer-
  Host-Papierkorb benutzt. Kein Shell-Handoff auf freiem Pfad, kein
  `trash::delete(path)`, kein Permanent-Fallback, keine unauffindbare Ersatzablage.
- **Fehlende Handoffdatei:** `api-delta/S-LOCAL.md` war beim gezielten Zugriff nicht
  vorhanden. Die konkrete Storage-Frage ist durch die schriftliche Parent-Auskunft
  geschlossen; daraus entsteht keine offene Implementierungsabhängigkeit.

Keine ungelöste fremde API-Grenze innerhalb des zugewiesenen Outcomes. Fehlende sichere
Provideridentität, externe Root-Verlagerung/Ersetzung, entfallene Ordinary-Rechte und
manipulierte Inhalte sind explizite Fehlerfälle, keine unsicheren Ausweichläufe.

## Dateibericht

Die tatsächlichen gelesenen Dateien, genau erstellten/geänderten Dateien, eigene
Self-Review-Ergebnisse und Befundzuordnung sind vollständig in
[abnahme/H-TRASH-WINDOWS.md](../abnahme/H-TRASH-WINDOWS.md#exakter-dateibericht) aufgeführt.
Exakte APIs, Registrierungen, neue Module und Testsymbole stehen im
[API-Delta](../api-delta/H-TRASH-WINDOWS.md).
