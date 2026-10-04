# Einmalige Plan-Kritik und eingearbeitete Entscheidungen

2026-10-04. Kritiker: `sync_plan_critic`, ausschließlich lesend. Keine
Codeänderung und keine Ausführung. Genau eine Runde; keine zweite Review.

Die Erstfassung war noch nicht abnahmefähig. Alle neun konkreten Lücken sind
angenommen und vor Code in den finalen Vertrag bzw. die Abnahme eingearbeitet:

| Befund | Entscheidung und konkrete Änderung |
|---|---|
| Temporäre Validierungsfehler verwerfen alte IDs | Tri-state: gültig, bestätigt anders/fehlend, nicht prüfbar. Letzteres propagiert als wiederanlaufbare Unterbrechung und ändert keinerlei Bindung. Frische 404-/Parent-/Typ-Evidenz ist nötig, bevor Abwesenheit zur Löschung beitragen darf. |
| Alte Jobs ohne eindeutige Evidenz | Die Migrationstabelle in `umsetzung.md` benennt jede Quelle und Entscheidung. Bestehende valide Cache-ID gewinnt; ohne Evidenz ist eine historische Auswahl bei echten Duplikaten nicht bewiesen. Bestehender Ordnerpicker ermöglicht eine ausdrückliche genaue Root-Auswahl; kein neues willkürliches Geschwister ersetzt einen bewiesen gebundenen Root. |
| Datei-/Ordner-/Literalnamenvertrag unklar | Dateien behalten Literal-Locators und Metadaten-IDs. Nur unabhängige Ordnerbäume erhalten projizierte Schlüssel/Marker-Locators. Sync-Stat und Child-Auflösung geben durchgehend die registrierte logische Zuordnung weiter; der Dateipublisher bekommt keine neue Marker-Destination. |
| Cross-Drive und Zielnamen unklar | Ein auf einer anderen Gegenstelle erzeugter Markertext ist ein Literalname, bis die dortige Registry eine andere Herkunft beweist. Projizierte Aliase bleiben reserviert, auch wenn ein Geschwister fehlt. TargetLimits und der vorhandene geschützte Auslassungsvertrag gelten für physisch unrepräsentierbare Namen; solche Pfade dürfen nicht als vollständiger Erfolg ausgegeben werden. Vollständige Hin-/Rückläufe sind C02/C04 zugeordnet. |
| Konkurrenz und beschädigte Persistenz | Versionierte private Records je Account/Parent, Processmutex + private Dateisperre, unter Sperre neu laden und zusammenführen, durable atomic write vor Verwendung. Unbekanntes/beschädigtes vorhandenes Format und Schreibfehler blockieren Mutation dieses Baums, erhalten Daten und die vorherige Datei. |
| Paging nicht gleich Abwesenheitsbeweis | Query-Corpus wird nach incompleteSearch gezielt eingegrenzt; zyklische/rejected Tokens verwerfen den gesamten Versuch. Die bestehende zeitlich begrenzte Backoff-/Retry-Politik gilt. Fehlende vorher erfasste IDs werden frisch überprüft; eine fehlende Prüfbarkeit ist keine Löschung. |
| Altversions-/Android-Proof fehlt | C08 startet die veröffentlichten gehashten v0.5.169-Workerbytes, lässt sie einen gespeicherten Job ausführen und einen realen Zustand erzeugen. Derselbe Profilezustand wird an den Kandidaten übergeben. C09 installiert die alte veröffentlichte APK und übernimmt dieselben Daten in eine inkrementelle, gleich signierte Development-APK; JNI/Worker-Neustart wird auf dem Gerät nachgewiesen. |
| Konkrete Fixtures/Live-Drive fehlen | `abnahme.md` ordnet jede Klasse benannten Fällen, Fixtures, Runtimeinputs und Orakeln zu. C10 verlangt einen isolierten echten Google-Drive-Lauf. Die Secrets-Metadaten wurden gelesen: vorhanden sind nur Android-Signatursecrets; Drive-Autorisierung fehlt. Textfrage an den Nutzer ist gestellt, übrige Arbeit läuft weiter. Ohne C10 gibt es keine behauptete Live-Drive-Abnahme und keinen abgeschlossenen Batch. |
| M5 verlangt schon Release aus M6 | M5 bestätigt ausschließlich S1–S8/C01–C10. S9 gehört ausschließlich zu M6 nach der erfolgreichen Suite. |

## Finale Entscheidung

M1–M6 bleiben ein zusammenhängender Batch. Die Planregeln und erwarteten
Ergebnisse sind vor Umsetzung konkret; der Mainagent nimmt den korrigierten
Plan zur Umsetzung an. Die noch fehlende reale Drive-Autorisierung betrifft
die abschließende Remote-Abnahme, nicht die erlaubte Implementierung.

Die sichtbare Präzisierung betrifft nur einen tatsächlich mehrdeutigen alten
Root ohne irgendeine historische Identitätsevidenz: dort wird die genaue
bestehende Ordnerauswahl benutzt. Beim gemeldeten eindeutigen `Notebook`
entsteht keine neue Bedienpflicht. Schutz, Berechtigungen und vorhandene
destruktive Zulassungen werden nicht erweitert.

Gelesen durch den Kritiker: die drei ursprünglichen Plandokumente,
`docs/ARCHITEKTUR.md`, Drive-Name-/ureq-/Remote-Suite-Refs sowie aktuelle
`resolution.rs`, `sync_listing.rs`, VFS `extensions.rs` und `apply_guard.rs`.
Angelegte/geänderte Dateien durch den Kritiker: keine.
