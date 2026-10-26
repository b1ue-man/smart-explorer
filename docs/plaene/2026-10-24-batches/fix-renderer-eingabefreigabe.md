# F3-Fix: Bildfehler entziehen Eingabe und SAS

Stand: 2026-10-26. Ausgangskandidat: `f120f910`. Dieser Bericht stammt vom Fixautor; der unabhängige Fehlerbefund bleibt in [review-login-bildspeicher.md](review-login-bildspeicher.md). Keine Build-, Test-, GPU- oder SAS-Ausführung durch diesen Autor.

## Auftrag und Erstplan

Quelle: delegierter Ergebnisauftrag des Hauptagents vom 2026-10-26 einschließlich der Nutzervorgabe, Funktionsauflistung, vollständige direkt/indirekt betroffene Verkettung, Fixplan, Umsetzung und eigene Anti-Regressionsprüfung durch denselben Fixautor zu liefern.

Fehlerhafte Endnutzerfunktionen: Nach einem fehlgeschlagenen Bildwechsel im Desktop-Anmeldefenster bleiben Tastatur-, Text- und Mauseingaben sowie die Schaltfläche „Strg+Alt+Entf anfordern“ durch einen früher gezeichneten Frame berechtigt. Die gemeinsame Login-Session akzeptiert zudem Touchereignisse über ihre bestehende Eingabeschnittstelle. Die Rechte des authentifizierten Kontos bleiben erforderlich; der Fehler betrifft den lokalen Bildbeleg. Eine Kontextquarantäne muss diesen Beleg ebenfalls entziehen. Die normale Bildschirmsteuerung teilt die Frame-/Renderergrenze und muss ihren Eingabeschutz bei Bildverlust behalten.

Aus dem Auftrag bekannt: A ist bereits gezeichnet; B überschreibt bei gleicher Größe dieselbe Textur; Upload oder Draw scheitert. Der Renderer entfernt den Resident, während A im Login-Guard gültig bleibt. SAS wird vor `LoginImage::show` verarbeitet. Aus dem Auftrag abgeleitete notwendige Korrektur: Der tatsächliche Fehlerpfad muss den Beleg synchron entziehen, unabhängig von einem folgenden UI-Durchlauf.

Erstplan: (1) Fehler bis zur Input-/SAS-Queue und Releasegrenze verfolgen; (2) Renderer- und Loginepoch gezielt koppeln; (3) alte Callbacks, reguläre Wiederherstellung und Ressourcenretirement prüfen; (4) fokussierte kontrollierte Fälle für die gemeinsame Remote-Suite vorbereiten. Erhalten bleiben vollständiger Clip, Fensterfokus, Bildwidgetfokus für normale Eingabe, SAS ohne gestohlenen Bildwidgetfokus, neun tatsächlich gezeichnete Guardbelege im Normalbetrieb, gleiche Größen, Größenwechsel, Kontextwechsel, lokale Claims und native Retirementbelege.

## Bestand, Funktionskette und erster Umsetzungsplan

Der vorgefundene Pfad lautet vollständig innerhalb der betroffenen Grenze:

| Bedienfunktion oder Grenze | Direkt und indirekt verkettete Stellen | Erhaltener Vertrag |
|---|---|---|
| Anmeldebild übernehmen und skalieren | `ViewerSession::take_frame` → `LoginImage::show`/`stamp` → `CpuTexture::update_local_frame`/`reframe`/`prepare` → `Worker` → `PreparedImage`/`Package` → `RenderSlot` | Raw-/Farb-/Skalierungsclaims bleiben an ihren tatsächlichen Besitzern; keine zusätzliche Bildkopie für den Entzug. |
| Upload und tatsächliche Zeichnung | `RendererOwner::paint_cpu` → `GlowBackend::upload`/`draw` → `finish_glow_upload`/`finish_glow_draw`; Fallback zeichnet denselben Resident über `draw_fallback` | Fehler erzeugen keine Receipts; die eigene Textur wird über den vorhandenen Retirementweg entfernt. |
| Gespeicherter Bildbeleg | `PresentedFrame::validity` → `LoginImage`-Receipt → `ViewerSession::presented` → `ViewerGuard::presented`/`shown`/`epoch` → `refresh`/`ViewerSnapshot` | Der aktive tatsächliche Fehler entzieht bereits bestätigtes A, nicht nur den B-Receipt. Alte Epochs können einen neuen Beleg nicht entfernen. |
| Normale Login-Eingabe | `LoginImage::show` → `InputMapper`/`Geometry` → `ViewerSession::input` → `Command::Input` → `run` → `exchange` → `LoginViewer::command` | Fokus, Steuern, Rechte, Geometrie, Frame und Epoch bleiben erforderlich. Eingereihte alte Befehle werden verworfen, signierte Befehle nicht wiederholt. |
| Sichere Aufmerksamkeit | `LoginViewerPane::show`-Schaltfläche vor `image.show` → `ViewerSession::secure_attention` → `Command::Sas` → `run` → `exchange` → `LoginCommand::SecureAttention` | Administrative SAS-/Login-/Steuerrechte bleiben erforderlich. Der Entzug gilt bereits vor einem folgenden `image.show`. |
| Loslassen und Abbruch | `invalidate_presentation`/`focus`/`release` → `Shared::release` → `run`/`ReleaseInputs`; `must_interrupt`/`input_flight`/`operating` → `close`/`LoginClose` | Gehaltene Eingaben werden freigegeben; eine überholte laufende Eingabe beziehungsweise SAS wird ohne automatischen Retry geschlossen. Bildplätze blockieren Release nicht. |
| Kontextquarantäne und Ressourcen | `GlowBackend::observe`/`checkpoint`/`service`/`retire` → `RendererOwner` → alle zugehörigen Slots → `Retirement`/`UploadTicket`/`Drain`; `shutdown`/`returned` | Alle betroffenen Bildbelege verlieren Gültigkeit. Unbestätigter Upload, Surface und beide CPU-Quellen bleiben bis zum tatsächlichen Drain oder konkreten Instanzende gebucht. |
| Normale Bildschirmsteuerung und Gruppen | `Shared::begin_frame`/`presented` → `PresentationState`/`ResumeGate` → `ViewerSession::input_ready` → `ViewWidget::input`/`family_viewer_session::input`; CPU- und YUV-Renderer teilen `Package`/`RenderSlot` | Bildverlust sperrt Eingabe; frischer Frame erhält neue Generation und übernimmt nur den bestehenden Visibility-Ack-Floor. Pause behält letzte Pixel ohne neuen Receipt. Releaseframes werden nicht durch einen neuen Sendefilter blockiert. |
| WGPU, Vorschau und Lebenszyklus | `RendererOwner::step` → `WgpuBackend::upload`; `CpuReceipt`/YUV-Callback; `screen_tile_face`/`screen_tile_grid`; `ScreenRenderApp` vor/nach App-Update und Rückkehr | Kein Umbau des Upload-/Sampler-/YUV-Vertrags, keine zusätzliche GPU-Behauptung, keine Änderung der nativen Ressourcenfreigabe. |

Der erste Umsetzungsplan aus dem Bestand ergänzt deshalb einen einmaligen Widerruf am Frame, eine epochengebundene Sessioninvalidierung, die SAS-Versandabsicherung, Quarantäne aller Slots sowie Screen-Generationserholung. Eine Änderung nur an `ready` scheidet aus, weil sie SAS vor `show` und die Queue nicht erreicht. Eine pauschale Schließung gesunder Sitzungen scheidet aus, weil vorübergehende GL-Fehler reguläre erneute Darstellung erlauben.

## Recherche und zweite Lückenrunde

Quellenprüfung am 2026-10-26: [Rust Atomic::swap](https://doc.rust-lang.org/std/sync/atomic/struct.Atomic.html) liefert den vorherigen Wert und unterstützt `AcqRel`; damit lässt sich der lokale Widerruf genau einmal auslösen und vor dem Sessioncallback sichtbar machen. Der Widerruf hält nur einen schwachen Sessionverweis. [Khronos glGetError](https://raw.githubusercontent.com/KhronosGroup/OpenGL-Refpages/main/gl4/glGetError.xml) unterscheidet ignorierte fehlerhafte Befehle von `GL_OUT_OF_MEMORY`, bei dem der GL-Zustand unbestimmt sein kann. Die bestehende Quarantäne und sichere Nichtfreigabe unbestätigter Drains bleiben Grundlage.

Die zweite Lückenrunde klärte drei Integrationsfragen: `ResumeGate::block` behält den Visibility-Floor und verhindert Bereitschaft bis zur frischen Zeichnung; eine entwertete Screen-Framegeneration muss in `begin_frame` ausdrücklich erneuert werden. `LoginImage::reframe` kann dieselben signierten CPU-Pixel nach einer neuen Epoch ohne Kopie erneut darstellen. `exchange` markiert bisher nur `Input` als eingaberelevant; SAS muss denselben Entzugs-/Close-Schutz erhalten, damit ein Widerruf zwischen Queueprüfung und Versand nicht übergangen wird.

## Finaler Meilensteinplan vor Produktänderungen

| Meilenstein | Dateien und Abhängigkeiten | Konkretes erwartetes Ergebnis |
|---|---|---|
| M1: Framewiderruf bis zum Login-Guard | `screen_render_upload.rs`, `login_viewer_image.rs`, `login/os/shared/viewer.rs`; vorhandener Guard und seine Epoch | Ein tatsächlicher Fehler entzieht alle alten A-Belege derselben aktiven Epoch. Stale Widerrufe ändern eine neue Epoch nicht. Input und SAS lehnen A auch ohne folgenden UI-Durchlauf ab. |
| M2: Upload-/Draw-/Quarantäneanschlüsse | `screen_render_cpu.rs`, `screen_render_glow.rs`, `screen_render_owner.rs`, `screen_render_upload.rs`; nach M1 | Fehler am Hauptdraw, Fallbackdraw, überschriebenen Upload und Quarantäne aller Slots erreichen M1. Kein Ressourcenclaim wird durch den Entzug vorzeitig freigegeben. |
| M3: Normale Wiederherstellung und Versand | `screen_presented.rs`, `screen_session.rs`, `login/os/shared/viewer.rs`; nach M1/M2 | Frische Screen-/Loginzeichnung bestätigt regulär neu. Alte Callbacks und Queuebefehle bleiben abgewiesen. SAS erhält den bestehenden In-flight-Entzugsschutz; Release bleibt unabhängig von Bildplätzen. |
| M4: Eigene Anti-Regressionsreview und vorbereitete Fälle | `batches_renderer_failure_task_tests.rs`, `batches_renderer_glow_task_tests.rs`, `batches_renderer_task_tests.rs`, bei Bedarf private Loginbildtests | Kontrollierte Fälle bilden A → bestätigter Draw → fehlerhaftes B → unmittelbare Input-/SAS-Abweisung, alte Callbacks, Neuerteilung, Multislotquarantäne, Screen-Floor und Retirements ab. Die vorhandenen Fokus-/Geometrie-/WGPU-Fälle bleiben eingebunden. Kein lokaler Lauf. |

Remote-Suite, Manifestregistrierung, Graphrefresh, Commit, Push und terminaler Release liegen beim Hauptagent. Vorbereitete Ereignisse und statischer Review sind keine ausgeführte GL-/WGPU-/SAS-Prüfung.

## Präzisierung aus der eigenen Anschlussprüfung

Vor Abschluss von M2 zeigte die erneute Lektüre von `GlowBackend::upload`: Ein Uploadfehler ist bereits vor `checkpoint` bekannt; dessen GL-ohne-Sync-Rückfall kann `glFinish` aufrufen. Ein Entzug erst in `finish_glow_upload` wäre deshalb zu spät während dieses Wartens. Nach ausdrücklich erweiterter Schreibfreigabe für `screen_render_glow.rs` löst `upload` den vorbereiteten Framewiderruf vor dem Checkpoint aus. `finish_glow_upload` bleibt eine zweite idempotente Absicherung vor dem Delete-/Retirementaufruf. Eine neu während `RendererOwner::step` erkannte Quarantäne beendet die weitere Slotbearbeitung und entzieht alle Kontextbelege, bevor der Owner weitere GL-Arbeit beginnt.

## Umsetzung und eigene Anti-Regressionsreview

Diese Prüfung stammt nach Fertigstellung der Produktänderungen vom selben Fixautor. Sie ist eine statische Anti-Regressionsreview des Diffs und der oben genannten Anschlüsse, kein zweiter unabhängiger Review und kein Laufzeitpass.

M1 ist im Quelltext umgesetzt: `Generation::revocable` setzt ihren Status atomar einmalig vor dem schwach gebundenen Sessioncallback zurück. `LoginImage::stamp` bindet den Widerruf an die genaue Epoch. `ViewerSession::renderer_failed` verändert nur die weiterhin aktuelle Epoch, entfernt über den unveränderten Guard alle bisherigen `shown`-/`presented`-Belege und setzt den bestehenden Releaseauftrag. Ein veralteter Widerruf verändert weder eine neue Epoch noch deren Releasezustand. FrameToken und Wireformat bleiben unverändert.

M2 ist im Quelltext umgesetzt: Uploadfehler widerrufen versuchte und überschriebene Frames vor Checkpoint und Retirement. Draw- und Fallbackfehler entziehen die Slotbelege vor Löschung. Quarantäne erfasst Kandidat, Resident, auslaufendes Uploadpaket und überschriebene Vorgänger sämtlicher Slots. Die Slotrunde endet bei neu erkannter Quarantäne vor weiterer GL-Arbeit. Der Widerruf bestätigt keinen Drain und gibt keine GPUressource vorzeitig frei.

M3 ist im Quelltext umgesetzt: `command_allowed` erhält die bisherigen Token-/Epoch-/Rechteprüfungen des Login-Workers. `begin_exchange` markiert auch `SecureAttention` als eingaberelevanten Austausch, sodass der bestehende Release-/Close-Schutz ein Rennen zwischen Queueprüfung und Versand erreicht. Bereits tatsächlich verschickte oder am Host ausgeführte Operationen lassen sich dadurch nicht zurücknehmen; eine solche Rücknahme wird nicht behauptet. Screen-`input_ready` verlangt eine aktive Präsentationsgeneration; `begin_frame` erneuert eine entzogene Generation und behält dabei den Visibility-Floor.

| Anti-Regressionsfrage | Tatsächlich nachgelesener Anschluss und Ergebnis | Beleggrenze |
|---|---|---|
| SAS vor `image.show` | Der separate SAS-Aufruf verwendet die inzwischen ungültige alte Sessionepoch und wird bereits dort abgewiesen. | Keine ausgeführte SAS-Operation. |
| Alte Callbacks und Wiederherstellung | CPU-, Glow- und YUV-Callbacks behalten Paket-/Frame-/Renderidentität. Alte Loginreceipts scheitern an der neuen Epoch; alte Widerrufe ändern die neue Epoch nicht. `reframe` verwendet dieselben CPU-Pixel erneut. | Kontrollierte Fälle vorbereitet, kein tatsächlicher Treiberlauf. |
| Fokus und Geometrie | Vollständiger Clip, Bild-/Clipvergleich, Fensterfokusflag und Bildwidgetfokus bleiben in `LoginImage::show` erhalten. SAS benötigt weiter keinen gestohlenen Bildwidgetfokus. Der bestehende Fokusfall und der neue Reframefall bleiben eingebunden. | Keine Interaktion mit einem echten Benutzerfenster. |
| Release, Queue und Abbruch | Der Worker prüft Token/Epoch/Rechte erneut; Release wird vor Capture-/Bildzulassung verarbeitet. Input und SAS setzen beide das In-flight-Flag. Der neue Mutexfehlerpfad droppt einen Poison-Guard vor `close` und verhindert dadurch eine rekursive Sperre. | Die Fixture prüft Entscheidungen und Flags; sie versendet keine privilegierten Befehle. |
| Screen, Gruppe und Vorschau | `ViewWidget::input` und `family_viewer_session::input` verwenden weiterhin `input_ready`. `ViewerSession::send` und `screen_decode_run::spawn_sender` bleiben unverändert, damit Releaseframes passieren. Pause und Visibility-Ack bleiben getrennt. | Bereits im Screen-Outbox befindliche Frames werden nicht rückwirkend entfernt. Kein neuer Screen-Wirevertrag. |
| Ressourcen und Ende | UploadStaging endet nach Uploadbestätigung; Surface und überschriebene Quelle bleiben bis zum Retirement. Quarantäne bestätigt keine künstlichen Drains; `shutdown` und konkrete `returned`-Grenze bleiben erhalten. | Physische GPU-Freigabe, OOM und echte Fencezustände ungeprüft. |
| WGPU und YUV | Upload, Registrierung, Submission, Polling, Sampler und YUV-Parameter bleiben unverändert. Standardgenerationen besitzen keinen Login-Widerrufscallback. | Vorhandene echte Headless-WGPU-Fälle müssen remote laufen. |
| Architektur | Alle geänderten Rustdateien liegen unter 500 Zeilen und 50 KiB. Keine neue Plattformabhängigkeit, Persistenz oder Berechtigungsregel. | Kompilierbarkeit bis zur Remote-Suite ungeprüft. |

Die eigene Prüfung hat zusätzlich den dokumentierten Checkpoint-Wartepfad geschlossen. Im abschließend gelesenen Diff bleibt kein konkreter ungelöster F3-Anschlussfehler innerhalb der zugewiesenen Fläche belegt. Ein noch nicht beobachteter Laufzeitfehler wird dadurch nicht ausgeschlossen.

## Genau vorbereitete Suitefälle

`batches_renderer_failure_task_tests.rs` ist direkt als Testmodul von `login_viewer_image.rs` eingebunden. `RendererFixture` liegt nur unter `cfg(test)` und benutzt produktiven Guard, Queueprüfung und Exchangeflags. Die Bildquelle stammt aus dem bestehenden echten IPC-Bildpfad. SAS-Rechte werden ausschließlich im danach getrennten kontrollierten Fixturekontext gesetzt; der IPC-Peer empfängt keine SAS- oder Eingabebefehle. Draw, Fehler, Release und Retirement sind kontrollierte Ereignisse.

| Fallname | Konkretes Orakel |
|---|---|
| `batches_task_b6_login_renderer_overwrite_revokes_input_sas_and_queue_before_retire` | A-Input, A-SAS, beide alten Queuebefehle und alter Receipt sind nach fehlerhaftem B bereits vor Retirement ungültig. Beide Quellen, Surface und Staging bleiben bis zu den passenden Drains gebucht. |
| `batches_task_b6_login_renderer_draw_failure_recovers_and_ignores_late_revocation` | Erfolgreicher B-Upload mit Drawfehler entzieht A. Frische Darstellung stellt Input/SAS her; alter Receipt und verspäteter Widerruf verändern den wiederhergestellten Zustand nicht. |
| `batches_task_b6_login_renderer_context_quarantine_revokes_every_slot_without_freeing_claims` | Zwei Login-Sessions verlieren alle Belege derselben Kontextquarantäne; Surfaceclaims bleiben bis Retirement bestehen. |
| `batches_task_b6_screen_renderer_failure_revokes_input_and_reuses_visibility_floor` | Legacy und moderner Screen sperren nach Bildverlust; neue Generation benötigt neue Präsentation und erhält den alten Visibility-Ack-Floor. |
| `batches_task_b6_login_renderer_failure_interrupts_input_and_sas_before_send` | Kontrolliert begonnener Input und SAS erreichen bei Widerruf den produktiven Closepfad. Kein privilegierter Befehl läuft. |
| `batches_task_b6_login_image_renderer_failure_reframes_same_pixels_before_sas_recovers` | Tatsächlicher `LoginImage::show`-Pfad mit kontrolliertem Draw entzieht alten SAS-Aufruf vor dem nächsten UI-Durchlauf und bestätigt nach Reframe dieselben Pixel neu. |
| `batches_task_b6_login_successful_overwrite_keeps_drawn_queue_proofs` | Erfolgreiche gleichgroße Folgeframes behalten den Neunerfenstervertrag; zehnter Frame und Resize erhalten die bisherigen Grenzen. |

Main bestätigte die Registrierung unter `R9-F3-Eingabefreigabe` im gemeinsamen Manifest. Vorhandene Glowfälle wurden ausschließlich an den Retirementcallback-Parameter angepasst. Bestehende Fokus-, Geometrie-, Preview-, Owner-, WGPU- und YUV-Fälle bleiben in der gemeinsamen Suite. Kein Fall wurde von diesem Autor ausgeführt.

## Gelesene Dateien

Die vollständige Liste umfasst gezielte Abschnitte und `rg`-Lektüre; sie behauptet keine Vollprüfung jeder Zeile. Repositorypfade sind relativ zur Repositorywurzel.

```text
docs/ARCHITEKTUR.md
docs/oberflaechen/bildschirm.md
docs/plaene/2026-10-24-batches/login-bildspeicher.md
docs/plaene/2026-10-24-batches/renderer-speicher.md
docs/plaene/2026-10-24-batches/review-login-bildspeicher.md
graphify-out/graph.json (query als Navigation)
native/Cargo.toml
native/Cargo.lock
native/review-task-native.py
native/review-task-manifest.json
native/src/app/core/batches_login_image_task_tests.rs
native/src/app/core/batches_preview_task_tests.rs
native/src/app/core/batches_renderer_glow_task_tests.rs
native/src/app/core/batches_renderer_task_tests.rs
native/src/app/core/batches_renderer_ui_task_tests.rs
native/src/app/core/batches_renderer_wgpu_task_tests.rs
native/src/app/core/family_viewer_session.rs
native/src/app/core/login_viewer.rs
native/src/app/core/login_viewer_image.rs
native/src/app/core/screen_decode_run.rs
native/src/app/core/screen_decode_thread.rs
native/src/app/core/screen_input_map.rs
native/src/app/core/screen_presented.rs
native/src/app/core/screen_render_app.rs
native/src/app/core/screen_render_cpu.rs
native/src/app/core/screen_render_glow.rs
native/src/app/core/screen_render_owner.rs
native/src/app/core/screen_render_upload.rs
native/src/app/core/screen_render_wgpu.rs
native/src/app/core/screen_render_yuv.rs
native/src/app/core/screen_session.rs
native/src/app/core/screen_texture_cpu.rs
native/src/app/core/screen_texture_worker.rs
native/src/app/core/screen_tile_face.rs
native/src/app/core/screen_tile_grid.rs
native/src/app/core/screen_view_widget.rs
native/src/app/core/screen_view_widget_render.rs
native/src/app/core/screen_viewer_view.rs
native/src/app/core/screen_visibility.rs
native/src/app/core/screen_wgpu_yuv.rs
native/src/app/screen_modules.rs
native/src/daemon/os/shared/batches_login_image_task_tests.rs
native/src/login/core/batches_authority_task_tests.rs
native/src/login/core/viewer_guard.rs
native/src/login/mod.rs
native/src/login/os/shared/viewer.rs
native/src/screen/core/input_model.rs
```

Zusätzlich gelesen: `/tmp/smartexplorer-renderer-input-revocation-fix-scope.json`, die beiden verfügbaren lokalen Arbeitsweise-SKILL.md-Fassungen und die Graphify-SKILL.md; aktuelle Arbeitsweise über den eigenen delegierten MCP-Kontext. Die eigenen neuen Dateien wurden ebenfalls nachgelesen. Erfolgreiche Webprimärquellen stehen im Rechercheabschnitt. Fehlgeschlagene AtomicBool-/Khronos-XHTML-Abrufe sind keine Belege. Graphify meldete eine Skill-/Paketversionsabweichung; der bestehende AST-Graph diente nur zur Navigation, ohne Graphrefresh.

## Erstellte und geänderte Dateien

Neu erstellt:

```text
docs/plaene/2026-10-24-batches/fix-renderer-eingabefreigabe.md
native/src/app/core/batches_renderer_failure_task_tests.rs
```

Geändert:

```text
native/src/app/core/batches_renderer_glow_task_tests.rs
native/src/app/core/login_viewer_image.rs
native/src/app/core/screen_presented.rs
native/src/app/core/screen_render_cpu.rs
native/src/app/core/screen_render_glow.rs
native/src/app/core/screen_render_owner.rs
native/src/app/core/screen_render_upload.rs
native/src/app/core/screen_session.rs
native/src/login/os/shared/viewer.rs
```

`viewer_guard.rs` und `screen_texture_cpu.rs` benötigten keine Änderung: ihre Invalidierungs-/Reframeverträge werden wiederverwendet. Das gemeinsame Manifest wurde nur von Main geändert; der unabhängige Reviewbericht wurde von diesem Autor nicht verändert.

## Offene Nachweise und Integrationsanschlüsse

Ausgeführt wurden Quellenlektüre, bestehender Graphquery und statische Text-/Diffprüfungen. `git diff --check` meldete für die freigegebenen bestehenden Änderungsdateien keinen Whitespacefehler. Kein Build, Compiler, Formatter, Test, Server, Paketinstall, Commit, Push, Graphrefresh oder Release wurde von diesem Autor gestartet.

Bei Main bleiben die gemeinsame Remote-Suite, die dadurch belegte Kompilierbarkeit und kontrollierte Funktion, Gesamtintegration, Root-Graphrefresh und terminaler Remote-Release. Echte GL-Upload-/Drawfehler, GPU-OOM, reale Treiberquarantäne und privilegierte SAS-Ausführung wurden nicht beobachtet. Fehlende echte GPU-/SAS-Belege sind kein Pass und nach Projektvorgabe kein eigenständiger Releaseblocker.

Dokumentationsanschluss außerhalb dieser Schreibfläche: `docs/oberflaechen/bildschirm.md` bezeichnet den F3-Restpfad noch als „noch zu korrigierend“. Main wurde über die konkrete Stelle informiert; ihre Aktualisierung darf nur Quellfix und tatsächlich vorhandene Nachweise behaupten.
