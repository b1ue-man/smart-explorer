# Google-Drive-Sync in Open-Source-Projekten – Vergleich mit Smart Explorer

Abgerufen 2026-10-08. Recherche ausschließlich lesend; Bewertung bewusst
ergebnisoffen (Nutzervorgabe: „es kann auch sein, dass sie nicht besser sind“).

## Quellen

- rclone `v1.75.1` (2026-09-04): `backend/drive/drive.go` (`listRGrouping = 50`,
  `listRRunner`, `ChangeNotify`/`changeNotifyRunner`, `defaultMinSleep = 100ms`,
  `defaultBurst = 100`, `createDir`, `PutUnchecked`), `lib/pacer/pacers.go`,
  `fs/march/march.go`, `cmd/bisync/*`, `docs/content/{drive,bisync}.md`.
- abraunegg/onedrive `v2.5.11` (2026-06-30): `src/sync.d` (Delta-Link erst nach
  vollständig verarbeitetem Durchlauf gespeichert), `monitor_fullscan_frequency = 12`.
- astrada/google-drive-ocamlfuse `v0.9.0` (2026-06-17): `driveMetadataRefresh.ml`
  (`probe_remaining_changes`, `change_limit = 50`).
- vitalif/grive2 master `be52cb21` (2024-07-26): `Syncer2::GetAll` (ganzes Konto je Lauf).
- Google Drive API v3, abgerufen 2026-10-08: guides/limits (Stand 2026-09-11),
  files.list und changes.list (2026-07-07), Change-Ressource (2025-04-18),
  File-Ressource (2026-07-14), manage-changes (2026-09-03), manage-uploads (2026-09-03).

## Befunde

1. Kein untersuchter Open-Source-Client synchronisiert Drive zweiseitig ohne
   vollständiges Einlesen je Lauf (rclone bisync: Listings beider Seiten +
   `.lst`-Vergleich; grive2: ganzes Konto). Smart Explorer arbeitet bei „Beide
   Richtungen“ gleich (vollständiger Scan + Sync-Basis).
2. rclone `fast-list` bündelt bis zu 50 Ordner-IDs pro `files.list`-Abfrage. Der
   veröffentlichte Faktor 20 entsteht überwiegend durch rclones eigene
   Standardbremse (10 Aufrufe/s); rclones bisync-Beispiel schaltet die Bündelung
   ab (`--disable ListR --checkers=16`). Google dokumentiert keine Höchstzahl von
   `in parents`-Klauseln; ein bekannter Google-Fehler (Issue 149522397) liefert bei
   gebündelten Abfragen manchmal leere Ergebnisse (rclone-Workaround).
   Kontingent seit 2026-05-01: 325 000 Einheiten/min/Nutzer/Projekt, `files.list`
   = 100 Einheiten → ≤ 3 250 Listings/min. Erst Bäume mit mehr Ordnern profitieren
   beim Kontingent.
3. rclone `ChangeNotify` ordnet Änderungen nur über bereits gecachte Ordner zu und
   verwirft den Rest; ocamlfuse verwirft oberhalb von 50 Änderungen den ganzen Cache.
4. rclone legt Ordner ohne vorab erzeugte ID an und wiederholt blind (Duplikate
   möglich, später `rclone dedupe`); Smart Explorer nutzt `generateIds`, ein
   prozessübergreifendes Anlagejournal und ID-genaue Prüfung.
5. Bestand Smart Explorer (geprüft 2026-10-08): gepoolte HTTP-Clients
   (`gdrive/core/http.rs`), `Retry-After` und Zufallsanteil in der Staukontrolle
   (`gdrive/core/overload.rs`), `incompleteSearch` wird als Fehler gewertet
   (`gdrive/core/file_list.rs`). Die ältere Lesung
   `docs/lesungen/2026-09-28-gdrive-transfer-cost.md` ist in diesen Punkten überholt.

## Bewertung und Entscheidung (Batch 2026-10-08)

| Technik | Quelle | Bewertung für Smart Explorer | Entscheidung |
|---|---|---|---|
| Feed auf den Sync-Ordner filtern, Wegverschieben über bekannte IDs erkennen | rclone/ocamlfuse (Prinzip) | Eigener Filter präziser als beide (keine verworfenen Änderungen, keine Pauschal-Invalidierung) | umgesetzt (`gdrive/core/change_scope.rs`) |
| Vollständigen Feed als Ereignisabdeckung werten, kein blinder 300-s-Volllauf | onedrive (Delta statt Scan) | Größter Gewinn bei wenigen Änderungen | umgesetzt (`daemon/os/shared/realtime.rs`), nur „Meine Ablage“ |
| HTTP-Verbindungen wiederverwenden | ureq | bereits vorhanden | keine Änderung |
| Jitter / `Retry-After` | rclone pacer | bereits vorhanden, ohne künstliche 10/s-Bremse | keine Änderung |
| Gebündeltes Listing (50 Ordner) | rclone ListR | Latenz bei kleinen/mittleren Bäumen gleichwertig; Kontingentvorteil erst > 3 250 Ordner | nicht umgesetzt; Messgrundlage liefert jetzt das Live-Protokoll (Dauer je Ordner) |
| Inkrementeller Zwei-Wege-Planer mit gespeichertem Feed-Token | onedrive | würde Smart Explorer vor alle Drive-Open-Source-Clients stellen; eigener Umbau | offen (TODO `GDRIVE-INCR`) |
| Multipart-Upload ≤ 5 MB mit vorab erzeugter ID | rclone/Google | spart 1–2 Aufrufe je neuer kleiner Datei, nur Übernahmephase | offen (TODO `GDRIVE-INCR`) |

Grenze des Filters: Das Entfernen eines gleichnamigen Duplikats im Sync-Ordner,
das kein Listing einem Pfad zuordnen konnte, erkennt erst der nächste Kontrolllauf.
