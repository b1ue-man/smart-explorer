# E-ENGINE – Anfragen und Grenzen

Stand: 2026-10-03. Produktanschluss abgeschlossen. [Fundzuordnung, konkrete Remote-Signale und das vollständige Datei-Inventar](../abnahme/E-ENGINE.md); [tatsächliche API-Änderungen](../api-delta/E-ENGINE.md).

Keine offene Produkt-/Scope-Anfrage. Die freigegebenen Read-/Modify-/Create-Grenzen für Identitätsaliase, Feedindex, typed Stage-Bindung, Merge-/Restore-Bindung und pairweiten Pending-Schutz sind umgesetzt. Kein Provider- oder Wireumbau nötig; bestehender VFS-Hook einschließlich H-REPLACE `reversible_replace_v1` wird konsumiert.

Beibehaltene praktische Grenzen:

- Accountfeed ohne beweisbare Root-Ancestry führt zum vollständigen Kontrollwalk. Unknown ist kein vollständiges Rootdelta; wiederholte IDs oder Dupegruppen werden nicht zusammengeklappt.
- Mehrere historische Baselinefamilien oder abweichende bereits vorhandene Zielbytes benötigen explizite Zustandsklärung. Die Migration erhält die alten Daten und blockiert, statt Owner/Replika/Baselines umzudeuten.
- Hook-`Ok(false)` belegt nur fehlende Hookmutation. Danach wird der bestehende Sync-Publish-Aufruf genau einmal ausgeführt; nach dessen Err/Lost ACK gibt es keinen weiteren Fallback. Wenn auch diese Backendprimitive Unsupported ist, bleibt die Ausführung sicher verweigert. FTP/DAV-Laufzeitkompatibilität ist ein konkretes Signal der gemeinsamen Remote-Suite, noch kein ausgeführter Nachweis.
- Fremde neue Destination-/Stage-/retained-Bytes bleiben erhalten; ungeklärte Recovery blockiert den normalen Lauf. Recovery bestätigt keine Baseline durch forget oder bloßen Neuvergleich.
- Low-level-Replacement ohne exakten RunVersions/VersionSide-Kontext erhält keinen globalen/geratenen Intent. Bestehende öffentliche Signaturen bleiben kompatibel.

Hauptagent übernimmt jetzt Produktcommit, gemeinsame Registrierungen/Graph und danach den gesondert freizugebenden Fixture-Anschluss. Die eigene zwischenzeitlich erstellte Fixture-Datei wurde vollständig entfernt; keine Fixture-Registrierung bleibt im Worktree. Die eine finale Remote-Suite muss besonders beide Lost-ACK-Publishzweige, FTP/DAV-Overwrite, Migration alter Versionen sowie Pending-Schutz anderer Owner/beider Orientierungen belegen. Keine Suite ausgeführt.
