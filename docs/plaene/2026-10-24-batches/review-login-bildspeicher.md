# Statischer Review C10 und L1–L8: Loginbildbesitz

Stand: 2026-10-26. Repository: /home/CodeProjects/smartexplorer/smart-explorer.

## Ergebnis und Beleggrenze

Im Ausgangsstand wurden drei konkrete Fehlergruppen belegt: nicht transaktionale Android-Besitzübergaben bei Kotlin-Allokationsfehlern (F1), erneute Präsentationsbelege nach Fensterfokusverlust auf Android und Desktop (F2), sowie erfolgreiche Glow-Präsentationsbelege trotz nicht propagiertem GL-Uploadfehler (F3). Die von Main geschriebenen Produktkorrekturen für F1 und F2 einschließlich der ergänzten Desktop-Fokusfixture sind inzwischen statisch nachgelesen. Glow-F3 bleibt im zuletzt gelesenen Stand offen. Der Nachtrag nennt die tatsächlichen Anschlussstellen.

Dies ist ausschließlich ein statischer Code- und Primärquellenreview. Es wurden keine Builds, Compiler, Formatter, Tests, Server, Installationen, Git-, CI- oder Releaseaktionen ausgeführt. Ein bestehender Graph wurde einmal mit graphify query gelesen; kein Graph wurde neu erzeugt. Keine Laufzeitfunktion und keine Kompilierbarkeit wird als nachgewiesen ausgegeben.

Main meldete als gepushten Integrationsstand 7ae228ff; diese Gitangabe wurde wegen des Verbots eigener Gitaktionen nicht selbst verifiziert. Die überprüften Dateien stammen aus dem gemeinsamen Arbeitsbaum. Der parallel geschriebene Capture-/Worker-/PipeWire-Anschluss ist nur eine APIreferenz, kein Beleg abgeschlossener Hostintegration. Die beiden Fehlerstellen F1/F2 waren vor den angekündigten Korrekturen in den unten genannten Zeilen tatsächlich vorhanden.

## Auftrag, Plan und Umfang

Quelle ist der delegierte Ergebnisauftrag einschließlich Nutzerwortlaut: „aufgrund des umfangs der aufgaben, ohne dass das weiteres vorgehen blockt, will ich dass du danach n astra xhigh die sachen kritisch reviewn lässt. per Batch“ und „also rein codeseitig und gegen quellen reviewen lässt“. Erlaubte Flächen und Ergebnisvorgaben stehen in /tmp/smartexplorer-login-image-review-scope.json. Produktänderungen sind nicht autorisiert.

Der erste Reviewplan aus dem Auftrag umfasste den zusammenhängenden Pfad Helper → Transport → PNG → Daemon/IPC → Viewer → Desktop-CPU/GPU beziehungsweise Android-JNI/HWUI. Die Bestandsaufnahme konkretisierte die Eigentümer als CaptureLifetime, ImageReceiver/ReceiveBuffers, Registry-Owner, ViewerFrame/CapturedImage, CpuPixels/UploadTicket/Resident sowie LoginBitmapTicket/BitmapOwner/LoginDrawBorrow/Target.

Der zweite Reviewplan prüfte vier konkrete Fragen: Besteht der Claim vor jeder produktseitigen Bildallokation? Überlebt er den tatsächlichen letzten Besitzer einschließlich Abbruch? Bleiben Release/Close und signierte Sequenzen von Bildrückstau unabhängig? Kann ausschließlich der passende aktuelle Zeichenpfad einen Präsentationsbeleg erzeugen? PNG/Tokio, jni, egui_glow/wgpu und AOSP wurden gegen die eingesetzten Versionen geprüft. Eine zweite gezielte Quellenrunde klärte die HWUI-Abbaugrenze, TextureView-Verbrauchsbestätigung und die fehlende GL-Fehlerweitergabe. Akzeptanz dieses Reviews ist ein belegter Befundbericht mit erhaltenen Verträgen und ausdrücklich offenen Grenzen; eine Laufzeitabnahme gehört nicht zu diesem Auftrag.

## F1 – Mittel: Kotlin-Allokationsfehler können gebuchte Androidbilder und Targets dauerhaft verwaisen lassen

**Konkreter Auslöser:** Nach einer erfolgreichen nativen Übernahme wirft die unmittelbar folgende Kotlin-Deserialisierung beziehungsweise Wrapperallokation eine Exception oder OutOfMemoryError. Es wird kein Auftreten dieses Fehlers behauptet; der Befund ist die statisch nachvollziehbare Fehlerfolge unter diesem ausdrücklich benannten Auslöser. Die Anwendung behandelt OutOfMemoryError im Bildempfänger ausdrücklich als beendbare Sitzung.

**Vor Korrektur gelesene Stellen:**

- android/app/src/main/java/app/smartexplorer/android/ui/login/LoginBitmapOwner.kt:58–64: loginBitmapTake veröffentlicht zuerst den Nativebesitzer; decodeFromString, require, LoginBitmapOwner und LoginPicture folgen ohne Rücknahme.
- Dieselbe Datei:71: owner.retain erhöht den Referenzzähler vor der LoginPicture-Konstruktion.
- Dieselbe Datei:89–98: calls.borrow erhöht den nativen Borrowzähler vor der Konstruktion von LoginDrawBorrow einschließlich AtomicBoolean.
- android/app/src/main/java/app/smartexplorer/android/ui/login/LoginBitmapRenderer.kt:52–56: claimTarget reserviert einen nativen Targetslot vor Target-Konstruktion sowie Registrierung in targets und retainedLoginTargets.
- Dieselbe Datei:77–78: picture.retain liegt vor Frame- und RectF-Konstruktion ohne Ausgleich bei Konstruktionfehler.
- native/android-bridge/src/login_bitmap_owner.rs:38–57, 59–89, 91–98: BITMAPS hält Global und Lease; TARGETS hält den RenderTargetclaim. Ein Bitmap wird erst bei retiring && borrows == 0 nach erfolgreichem recycle entfernt.
- native/src/mobile/os/shared/domains/login_images.rs:61–70: Pane-Close schließt die Session und belässt lebende Bild-/Targetclaims ausdrücklich in der Registry.
- android/app/src/main/java/app/smartexplorer/android/ui/login/LoginSession.kt:84–94, 125–139: Fehlerbehandlung kann nur bereits veröffentlichte Kotlin-Pictures schließen.

**Wirkung:** Der Nativeeintrag bleibt ohne erreichbaren Kotlinbesitzer bestehen oder sein Borrow-/Referenzzähler bleibt erhöht. Der GlobalRef hält das Bitmap gegen Garbage Collection; der Slice belegt weiterhin Rootbytes und einen der zwei Bitmapplätze. Ein verwaister Targetclaim belegt einen der zwei Targetplätze. Close/Reconnect behebt diesen Verlust nicht; ein Pane kann nach wiederholtem Fehler keine Bilder oder Targets mehr annehmen. Geschlossene Panes mit solchen Restclaims werden nicht aus der begrenzten Pane-Registry entfernt.

Die JNI-seitige Behandlung eines fehlgeschlagenen JString-Aufbaus in login_bitmap_jni.rs:24–31 ist vorhanden, erfasst aber keinen Fehler nach erfolgreicher Rückkehr nach Kotlin. Die zusätzlichen JSON-Felder stride/allocation_bytes sind kein regulärer Parsefehler: Core.json ignoriert unbekannte Schlüssel.

**Primärbeleg:** jni 0.22.4 beschreibt Global ausdrücklich als threadübergreifenden Besitzer, der Garbage Collection bis zu seinem Drop verhindert; lokale Fundstelle src/refs/global.rs:23–64. [jni 0.22.4 Global](/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/jni-0.22.4/src/refs/global.rs:23).

**Kleinster betroffener Vertrag:** Jede native Reserve-/Publish-/Borrowoperation muss bis zum erfolgreich registrierten Kotlinbesitzer ein exaktes Rollback besitzen. Rollback muss das konkrete Tupel owner/picture/generation beziehungsweise die konkrete Target-ID freigeben. Ein pauschales Retire gültiger Bilder beim Sessionwechsel wäre keine kompatible Behebung.

**Korrekturstand:** Die konkrete Fehlergruppe ist im nachgelesenen Arbeitsbaum statisch behoben; siehe Nachtrag. Ein ausgeführter Fehler-Injektionsbeleg liegt nicht vor.

## F2 – Mittel: Ein frischer Draw nach Fokusverlust kann die SAS-Präsentationsgrundlage wiederherstellen

**Android-Auslöser:** Das Fenster verliert Fokus, bleibt jedoch sichtbar. onWindowFocusChanged(false) ruft release/resetHardwareSurface auf und startet anschließend erneut submitHardware. Eine neue Surface und ein neuer Stamp entstehen nach der Invalidierung.

**Vor Korrektur gelesene Stellen:**

- android/app/src/main/java/app/smartexplorer/android/ui/login/LoginInput.kt:84–105, 122–125: fullyVisible und confirm prüfen Sichtbarkeit, Geometrie, Generation und Epoch, jedoch keinen Fensterfokus. Eine nach Fokusverlust frisch erzeugte Produktion erfüllt diese Prüfungen.
- android/app/src/main/java/app/smartexplorer/android/ui/login/LoginSession.kt:103–108: presented prüft aktuelle Epoch und Sessionende, jedoch focused nicht.
- native/src/login/core/viewer_guard.rs:42–54: presented nimmt passende Tokens an; action_ready enthält keinen Fokuscheck. input_ready ergänzt focused und control.
- native/src/login/os/shared/viewer.rs:119–125: secure_attention verwendet action_ready, Epoch und administrative Rechte.
- android/app/src/main/java/app/smartexplorer/android/ui/login/LoginViewScreen.kt:76–77 und LoginSession.kt:115: SAS bleibt ein eigener Aufruf; die normalen Inputprüfungen schützen diesen Präsentationsanschluss nicht.

**Desktop-Auslöser:** Das sichtbare Desktopfenster verliert Fokus. LoginImage.show ruft focus(false, ...) auf, reframed anschließend auf die neue Epoch und zeichnet weiter.

- native/src/app/core/login_viewer_image.rs:33, 42–50: focused wird ermittelt, aber der Receiptabschluss hängt anschließend nur von vollständigem Clip und Sessionidentität ab.
- Dieselbe Datei:66–71: die Generation prüft Epoch und Sessionende, nicht Fensterfokus.
- native/src/app/core/login_viewer.rs:72–74: SAS verwendet den vom Viewer gespeicherten presented_frame.

**Wirkung:** Nicht lediglich ein alter Callback, sondern ein neuer Callback der aktuellen Epoch kann nach Fokusverlust wieder einen shown-/action_ready-Beleg erzeugen. Dadurch ist die lokale Voraussetzung für SAS erneut erfüllt, obwohl der Fokusvertrag sie gerade entwertet hat. Gewöhnliche Tastatur-/Pointereingabe bleibt durch focused/control gesperrt. Maschinenpin, Challenge, Benutzerrechte und administrative SAS-Rechte werden durch diesen Befund nicht umgangen; eine tatsächlich ausgelöste SAS-Operation wurde nicht beobachtet.

**Vertragsbeleg:** login-bildspeicher.md:159 fordert Fokus zusätzlich zu Producer-/Consumerbeleg und Epoch; :173 erhält die Entwertung bei Fokusverlust. Die Androidplattform unterscheidet View-Sichtbarkeit und Fensterfokus; ein FrameCommit belegt zudem nur Einreichung in die Swapchain. [ViewTreeObserver FrameCommit](https://developer.android.com/reference/android/view/ViewTreeObserver#registerFrameCommitCallback(java.lang.Runnable)). AOSP android-16.0.0_r1 TextureView.applyUpdate ordnet den Updatecallback hinter das Einreihen der TextureLayer-Aktualisierung; der Callback allein beweist keinen fortbestehenden Fokus. [AOSP TextureView](https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android-16.0.0_r1/core/java/android/view/TextureView.java).

**Kleinster betroffener Vertrag:** Die Plattformrenderer dürfen nach Fensterfokusverlust keine neue Präsentationsgrundlage bestätigen. Auf Desktop müssen Fensterfokus und Bildwidgetfokus getrennt bleiben, damit ein Klick auf den administrativen SAS-Button nicht durch den eigenen Widgetfokuswechsel dauerhaft verhindert wird. Eine pauschale neue focused-Bedingung in action_ready wäre deshalb kein ungeprüft austauschbarer Fix.

**Korrekturstand:** Die Android- und Desktop-Produktkorrekturen sowie die korrigierte Kontrollfixture wurden tatsächlich nachgelesen; siehe Nachtrag. Keine Fixture wurde ausgeführt.

## F3 – Hoch: Glow meldet auch nach fehlgeschlagenem Upload einen erfolgreichen Bildbeleg

**Konkreter Auslöser:** Der GL-Treiber meldet beim Texturupload GL_OUT_OF_MEMORY oder einen GL-Fehler, der den Upload verwirft. Der zuvor erfolgreiche Rootclaim ist keine physische Garantie gegen Treiber-/GPU-Speichermangel.

**Gelesene Stellen:**

- native/src/app/core/screen_render_glow.rs:16–18: upload ruft den Painter auf und gibt unabhängig vom GL-Ergebnis GlowImage zurück.
- native/src/app/core/screen_render_cpu.rs:31–38: der Aufrufer installiert das Paket als Resident und ruft nach gl.draw bedingungslos den Receipt auf.
- /root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/egui_glow-0.29.1/src/painter.rs:510–543, 628–640: set_texture gibt kein Result zurück und tex_image_2d nutzt lediglich check_for_gl_error.
- Die gepinnte Macrodefinition in egui_glow 0.29.1/src/lib.rs:25–46, 68–104 loggt nur; der Check ist im Releasebuild deaktiviert. Im Debugbuild konsumiert er das Fehlerflag, ohne den Upload als fehlgeschlagen zurückzugeben.

**Wirkung:** Das neue Login-FrameToken kann presented und bei vorhandenem Fokus/Steuerrecht input_ready werden, obwohl der neue Bildinhalt nicht erfolgreich hochgeladen wurde. Der sichtbare Inhalt kann alt, leer oder bei GL_OUT_OF_MEMORY unbestimmt sein. Ein anschließend signalisierter GL-Fence bestätigt Befehlsende, nicht Uploadkorrektheit.

**Primärquellen:** [egui_glow 0.29.1 Fehlercheck](https://raw.githubusercontent.com/emilk/egui/0.29.1/crates/egui_glow/src/lib.rs); [Khronos glGetError](https://raw.githubusercontent.com/KhronosGroup/OpenGL-Refpages/main/gl4/glGetError.xml); [Khronos glFenceSync](https://raw.githubusercontent.com/KhronosGroup/OpenGL-Refpages/main/gl4/glFenceSync.xml). Khronos beschreibt das Zurücksetzen des Fehlerflags beim Lesen; fehlgeschlagene Befehle werden im Regelfall ignoriert, bei Out-of-memory ist der GL-Zustand abgesehen von Fehlerflags unbestimmt. Die Khronos-Referenzen sind der am 2026-10-26 abgerufene GL4-Referenzstand, kein gepinnter Projektcrate.

**Kleinster betroffener Vertrag:** Der gemeinsame Glow-Upload-/Drawanschluss muss seinen tatsächlichen Erfolg bis zur Residentübernahme und Receiptfreigabe melden. Nach erkanntem Fehler darf das neue Paket keinen Beleg erhalten. Ein alleiniger get_error-Aufruf nach dem bestehenden Painter ist für Debugbuilds unzureichend, weil dessen Macro den Fehler bereits gelesen haben kann. Dieser Befund betrifft den gemeinsam genutzten Renderer und ist keine Forderung nach einer separaten Loginrendererimplementierung.

**Korrekturstand:** An Main gemeldet; kein Fix als vorhanden angenommen.

## Statisch erhaltene Verträge

| Abschnitt | Im gelesenen Code erhaltener Anschluss | Präzise Grenze |
| --- | --- | --- |
| C10 Helper | ParentCapture reserviert vor dem produktiven Helferpfad; Helperquelle wird erst mit Grant aktiviert; Elterntransport und Quellenanteil verwenden getrennte Claims. CaptureLifetime trennt Reader-Ende und bestätigten Prozessausgang. Windows wartet auf signalisierten Prozesshandle; Linux prüft tatsächliches try_wait-Ergebnis. | Kein beobachteter Windows-/Linux-Prozesslauf. Fehler beim Kill/Wait bleibt absichtlich gebucht; das beweist sichere Nichtfreigabe, keine garantierte spätere Rückgewinnung. |
| L1 Admission | ImageAdmission::try_claim prüft Rollenanzahl, Rollenbytes, Gesamtbytes und denselben Rootpool vor Commit. Fehler vor Commit verändert die Ledger nicht. Slice-Splits geben ihre Byteanteile getrennt frei; der Rollenplatz lebt bis zum letzten Segment. observe_capacity_ceiling begrenzt aktive Kohorten ohne alte Besitzer künstlich zu löschen. | Der Pool ist Zusagebuchhaltung; kein physischer Reservierungsnachweis. Gleichzeitige Hostintegration wurde nicht als abgeschlossen bewertet. |
| L2 PNG | ImageReceiver::prepare beansprucht PNG P, BGRA N und RGB-Zeile vor Produktbufferallokation/PNG-Bodyread. RGB8, unabhängige nichtinterlacete PNGs, Dimensionen, Zeilenanzahl und Ende werden geprüft. Blockingclosure und Ergebnis halten Lifetimeanker. | Bibliotheksinterne Codec-/Allocatorstrukturen sind keine zusätzlich untersuchten Produktbuffer. Kein PNG-Fuzz- oder Laufzeitnachweis. |
| L3 Daemon/IPC | write_owned lässt das Bild vor der Abschlussmeldung fallen; run beginnt den nächsten signierten Vorgang erst nach Abschluss. Registry-Owner reicht bis Decoderclosure und Ergebnis. read_pixels liest geprüfte Chunks direkt ins zugelassene Ziel. | Kein ausgeführter Socket-/Writerabbruch. Die geschriebenen Fixtures sind Quellenbeleg für die beabsichtigte Grenze. |
| L4 Viewer | Zwei wirkliche Rawplätze begrenzen Produktion; entnommene CapturedImage-Aliase bleiben gebucht. Release/Close setzen Kontrollzustand und Socketshutdown unabhängig von Bildplätzen. Kein neuer automatischer Retry signierter Eingaben. FrameToken, Kontext und Epoch bleiben im Guard. | F2 wurde an den Plattformaufrufern statisch nachgebessert. Der Guard allein wäre kein Ersatz für diese Prüfung. |
| L5 Desktop-CPU | Reale Viewerimages gehen in die gemeinsame CpuTexture. ColorSource, ScaledOutput und FilterScratch werden getrennt vor Allokation beansprucht. Worker hält Quelle und unveröffentlichtes Resultat; Reset ersetzt den Paneledger nicht. | Keine ausgeführte Skalierung, insbesondere keine gemessene 4K-Speicherspitze. |
| L6 Desktop-GPU | UploadStaging und RenderSurface besitzen getrennte Claims. WGPU schreibt über Queue, registriert Completion und betreibt Polling. Surfaceclaims verbleiben über Uploadende bis Retirement. Glow besitzt Painter-/Fence-/Shutdownlebensdauer. | F3 lässt GL-Fehler derzeit als Erfolg passieren. Ein Rendercallback ist kein Nachweis physischer Bildschirmanzeige. |
| L7 Android-JNI | Javaheapangebot gelangt vor Open in den gemeinsamen Nativepool. Pane überlebt Sessionversuche. Natives Ticket hält Rawimage während NDK-Fill; RGBA-Format, Dimensionen und tatsächlicher Stride werden geprüft. BGRA→RGBA erfolgt zeilenweise ohne vollständiges IntArray/DirectBuffer. Global und Slice bleiben bis retire+letztem Borrow erhalten. | F1 statisch nachgebessert; keine ausgeführte JNI-/NDK-Prüfung. getAllocationByteCount wird diagnostiziert; Payloadclaim und physische Allokationsgröße sind nicht dasselbe. |
| L8 Android-HWUI | Eigener HardwareRenderer/RenderNode; Quelle gelangt nicht in die normale View-Displayliste. Ein laufender Frame, ein vorgemerkter Frame. Producercommit, SurfaceTimestamp und nachfolgender Fenstercommit werden zugeordnet. clearContent/discard/destroy laufen vor Borrow-/Targetfreigabe. Echter Softwarecanvas schließt seinen Borrow nach drawBitmap. | F1 und Android-F2 statisch nachgebessert; kein nachgewiesener OEM-/GPU-Lauf. Abbaufehler behalten den gebuchten Besitzer und blockieren neue Slots. |

Tokio 1.52.3 dokumentiert ausdrücklich, dass ein bereits gestartetes spawn_blocking nicht durch Abbruch des JoinHandles beendet wird; die Besitzübernahme in die Closure ist daher erforderlich und hier vorhanden. [Tokio 1.52.3 blocking.rs](https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.52.3/tokio/src/task/blocking.rs). png 0.18.1 bietet den verwendeten Zeilendecode und finish-Abschluss. [png 0.18.1 Reader](https://docs.rs/png/0.18.1/png/struct.Reader.html).

WGPU 22.1.0/src/lib.rs:5475–5568 bestätigt die sofortige Kopie von write_texture in Staging, Start mit dem folgenden Queue::submit und Completion nach vorangegangener Submission. Der gelesene Code hält Claims mindestens bis zu diesen Ereignissen. [wgpu 22.1.0 Queue](https://docs.rs/wgpu/22.1.0/wgpu/struct.Queue.html).

Für minSdk 30 wurden AOSP android-11.0.0_r1 und android-16.0.0_r1 abgeglichen: clearContent entfernt Displaylisten; RenderProxy::destroyHardwareResources führt den Abbau synchron auf dem RenderThread aus; destroy synchronisiert vor Aufgabe der Surface. Das stützt die gewählte Besitzergrenze, ersetzt aber keinen allgemeinen GPU-Fence oder OEM-Nachweis. [Android 11 HardwareRenderer](https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android-11.0.0_r1/graphics/java/android/graphics/HardwareRenderer.java), [Android 11 RenderProxy](https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android-11.0.0_r1/libs/hwui/renderthread/RenderProxy.cpp), [Android 16 RenderProxy](https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android-16.0.0_r1/libs/hwui/renderthread/RenderProxy.cpp), [Android 16 RenderNode](https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android-16.0.0_r1/libs/hwui/RenderNode.cpp). Die NDK-Lockgrenze entspricht dem dokumentierten Zugriff auf Bitmap-Pixelspeicher. [NDK Bitmap](https://developer.android.com/ndk/reference/group/bitmap).

## Offene Grenzen und Entscheidungen

1. Die vorhandene Kotlin-HWUI-Fixture LoginImageTaskTest verwendet simulierte LoginBitmapCalls. Sie zeichnet mit HardwareRenderer/TextureView, ruft jedoch keine produktive JNI-/NDK-/GlobalRef-Grenze auf. Die Rustfixture unter mobile/os/shared/domains prüft reale Slices und einen strided Zielslice, aber kein Java-Bitmap und keine JNI-Retirementfolge. Die von Main separat geplante JNI-Instrumentabnahme ist kein vorhandener Laufzeitbeleg.
2. Die vorhandene Fokusfixture überprüft alte Stamps nach release/focus/close. Sie deckt im ursprünglichen Stand keinen neu erzeugten Stamp bei fortbestehendem Fokusverlust ab; genau dieser Fall löst F2 aus.
3. Die Desktopintegration nutzt gemeinsame CPU-/GPUklassen; kontrollierte Rendererfixtures sind kein ausgeführter WGPU-/Glow-Zeichenpfad. Nach expliziter Erweiterung der Lesefläche wurde der reguläre Einstieg bestätigt: native/src/main.rs:8 → native/src/lib.rs::run_gui:200 → screen_render_app::run_native. screen_launch und family_viewer verwenden denselben Wrapper.
4. JNI-Exception-, NDK-Unlock-, recycle-, HWUI-Abbau-, GPU-Device-Loss- und GL-Out-of-memory-Pfade wurden nur statisch beurteilt. Keine der in diesem Bericht beschriebenen Faultbedingungen wurde praktisch injiziert.
5. Aufnahme-/Rendererpläne bleiben der Maßstab. Neue Core-/Worker-/PipeWirefassaden im Arbeitsbaum wurden ausdrücklich nicht als fertig integrierte Hostfunktion ausgegeben. Screen-v2-W/24 und PacketBudget wurden nicht über eine zweite Loginverhandlung ersetzt; ein vollständiger Screen-v2-Regressionsnachweis liegt in diesem Review nicht vor.
6. Entsprechend dem Auftrag wurden keine Reparaturen ausgeführt. Die Entscheidung, die drei konkreten Fehlergruppen an Main zu melden, erfolgte vor Berichtsabschluss. Eine fehlende breite Laufzeitabnahme wurde weder als bestandene Abnahme noch als eigener Releaseblocker ausgegeben.
7. Webabrufe der Android-View-Referenz scheiterten an ihrer Größe; Khronos-XHTML scheiterte am Werkzeug-Inhaltstyp. Für GL wurden die erfolgreichen primären Khronos-XMLquellen verwendet. Kein fehlgeschlagener Abruf dient als technischer Beleg.

## Nachtrag: tatsächlich gelesener Korrekturstand

F1: LoginBitmapOwner.kt:58–70 ruft bei jedem Throwable nach nativer Veröffentlichung loginBitmapDiscard(handle, raw) auf; NativeBridge.kt:61 und login_bitmap_jni.rs:34–41 stellen diesen Anschluss bereit. login_bitmap_owner.rs:78–83 prüft den übergebenen Paneowner und retiert ausschließlich das konkrete picture/generation-Tupel. LoginBitmapOwner.kt:79–82 und :101–105 gleichen fehlgeschlagene Picture-/Borrowkonstruktion aus. LoginBitmapRenderer.kt:52–63 nimmt Targetclaims bei Konstruktion-/Registrierungsfehler zurück; :85–87 schließt das zuvor retained Bild bei fehlgeschlagenem Framebau. Die gemeldeten Besitzübergänge sind damit statisch geschlossen. Die Fehlerbehandlung kann keinen Erfolg eines seinerseits fehlgeschlagenen JNI-/recycle-Aufrufs garantieren; dann bleibt die bestehende sichere Nichtfreigabe bestehen. JString::try_to_string ist in der eingesetzten JNI-Version vorhanden. [jni 0.22.4 JString](https://docs.rs/jni/0.22.4/jni/objects/struct.JString.html#method.try_to_string).

F2 Android: LoginInput.kt:88 prüft zusätzlich hasWindowFocus; LoginSession.kt:104 lehnt presented bei !focused ab. Ein neuer Stamp nach Fokusverlust durchläuft damit keine erneute Präsentationsfreigabe. Die Prüfung hängt nicht vom Steuerrecht ab und erhält reine Ansicht. Diese Feststellung ist statisch, ohne ausgeführte JNI-/HWUI-Prüfung.

F2 Desktop: login_viewer_image.rs:34–36 trennt Fensterfokus von Bildwidgetfokus. Jeder Fensterfokuswechsel invalidiert die Präsentation; :49–53 erzeugt neue Receipts nur bei Fensterfokus und prüft das geteilte window_active-Flag erneut im Callback. Reset, unsichtbares Fenster, Sessionende und unsichtbare Bildfläche setzen dieses Flag zurück. Damit sind sowohl alte Closures als auch frisch erzeugte unfokussierte Draws am gemeldeten Anschluss gesperrt, ohne den SAS-Button an Bildwidgetfokus zu binden.

Die gleichzeitig ergänzte Kontrollfixture war zunächst unvollständig: reframe ruft CpuTexture.reset auf, entwertet den Resident und stellt einen neuen Kandidaten ein; ControlledDriver::draw_output (batches_renderer_task_tests.rs:27) führt nur vorbereitete Callbacks aus. Nach Meldung an Main enthält login_viewer_image.rs:107–113 jetzt after_update, complete_retirement, upload, complete_upload und einen erneuten show-Durchlauf vor dem Refokusbeleg. Damit ist die konkret fehlende Ereignisfolge in der Fixture ergänzt. Kein Testlauf fand statt.

F3 Glow: Die angekündigte Korrektur ist noch nicht nachgelesen; der Befund bleibt offen.

## Vollständige Liste der berührten lokalen Lesepfade

Die Liste umfasst vollständig gelesene Dateien und gezielt per rg/Abschnitt gelesene Dateien; sie behauptet keine Vollprüfung jeder Zeile jeder Datei. AGENTS.md wurde als Nutzeranweisung geliefert. Der Graph wurde nur als Navigationsindex abgefragt.

Auftrags-/Anleitungsquellen:

- /tmp/smartexplorer-login-image-review-scope.json
- /root/.codex/skills/graphify/SKILL.md
- Arbeitsweise: MCP skill_message/skill_read/skill_decide; keine zusätzliche Produktdatei.

Repositorypfade relativ zu /home/CodeProjects/smartexplorer/smart-explorer:

```text
docs/ARCHITEKTUR.md
docs/plaene/2026-10-24-batches/login-bildspeicher.md
docs/plaene/2026-10-24-batches/aufnahme-speicher.md
docs/plaene/2026-10-24-batches/renderer-speicher.md
docs/plaene/2026-10-24-batches/umsetzung.md
docs/plaene/2026-10-24-batches/review-b6.md
docs/plaene/2026-10-24-batches/review-b7.md
graphify-out/graph.json
native/Cargo.toml
native/Cargo.lock
native/src/main.rs
native/src/lib.rs
native/android-bridge/Cargo.toml
native/android-bridge/src/lib.rs
native/android-bridge/src/login_jni.rs
native/android-bridge/src/login_bitmap_jni.rs
native/android-bridge/src/login_bitmap_owner.rs
native/android-bridge/src/login_bitmap_pixels.rs
native/src/app/mod.rs
native/src/app/screen_modules.rs
native/src/app/core/init.rs
native/src/app/core/family_viewer.rs
native/src/app/core/screen_launch.rs
native/src/app/core/screen_viewer_app.rs
native/src/app/core/frame_update.rs
native/src/app/core/login_viewer.rs
native/src/app/core/login_viewer_image.rs
native/src/app/core/screen_image_memory.rs
native/src/app/core/screen_texture_cpu.rs
native/src/app/core/screen_texture_worker.rs
native/src/app/core/screen_scaling.rs
native/src/app/core/screen_presented.rs
native/src/app/core/screen_render_upload.rs
native/src/app/core/screen_render_cpu.rs
native/src/app/core/screen_render_owner.rs
native/src/app/core/screen_render_wgpu.rs
native/src/app/core/screen_render_glow.rs
native/src/app/core/screen_render_app.rs
native/src/app/core/screen_view_widget_render.rs
native/src/app/core/batches_login_image_task_tests.rs
native/src/app/core/batches_renderer_task_tests.rs
native/src/daemon/os/shared/login_ipc_session.rs
native/src/daemon/os/shared/login_ipc_registry.rs
native/src/daemon/os/shared/login_ipc.rs
native/src/daemon/os/shared/login_ipc_client.rs
native/src/daemon/os/shared/ipc_protocol_login.rs
native/src/daemon/os/shared/batches_login_image_task_tests.rs
native/src/login/core/gateway_wire.rs
native/src/login/core/image_receive.rs
native/src/login/core/image_send.rs
native/src/login/core/capture_wire.rs
native/src/login/core/viewer_guard.rs
native/src/login/core/batches_capture_task_tests.rs
native/src/login/core/batches_image_receive_task_tests.rs
native/src/login/os/shared/viewer_images.rs
native/src/login/os/shared/viewer.rs
native/src/login/os/shared/gateway_client.rs
native/src/login/os/shared/client.rs
native/src/login/os/shared/capture_admission.rs
native/src/login/os/shared/capture_lifetime.rs
native/src/login/os/shared/child_reader.rs
native/src/login/os/shared/helper_capture.rs
native/src/login/os/shared/child.rs
native/src/login/os/shared/helper.rs
native/src/login/os/windows/spawn.rs
native/src/login/os/linux_os/spawn.rs
native/src/mobile/mod.rs
native/src/mobile/os/shared/domains/mod.rs
native/src/mobile/os/shared/domains/login_images.rs
native/src/mobile/os/shared/domains/login_view.rs
native/src/mobile/os/shared/domains/batches_login_image_task_tests.rs
native/src/screen/core/image_admission.rs
native/src/screen/core/frame_memory.rs
native/src/screen/core/capture_bytes.rs
native/src/screen/core/capture_grant_slices.rs
native/src/screen/core/capture_image.rs
native/src/screen/core/capture_source_contract.rs
native/src/screen/os/shared/image_memory.rs
native/src/screen/os/windows/login_source.rs
native/src/screen/os/windows/capture_slots.rs
native/src/screen/os/linux_os/login_source.rs
native/src/screen/os/linux_os/x11_admission_stream.rs
android/app/build.gradle.kts
android/app/src/main/java/app/smartexplorer/android/core/Core.kt
android/app/src/main/java/app/smartexplorer/android/core/NativeBridge.kt
android/app/src/main/java/app/smartexplorer/android/api/LoginApi.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenFrameLimits.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginBitmapOwner.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginBitmapRenderer.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginBitmapSurface.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginSession.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginInput.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginViewScreen.kt
android/app/src/androidTest/java/app/smartexplorer/android/task/LoginImageTaskTest.kt
android/app/src/test/java/app/smartexplorer/android/ui/login/LoginBitmapOwnerTest.kt
```

Lokal gelesene versionsgebundene Bibliotheksquellen:

```text
/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/jni-0.22.4/src/env.rs
/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/jni-0.22.4/src/errors.rs
/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/jni-0.22.4/src/refs/global.rs
/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/egui_glow-0.29.1/src/painter.rs
/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wgpu-22.1.0/src/lib.rs
```

Weitere erfolgreich abgerufene Webprimärquelle: AOSP android-16.0.0_r1/graphics/java/android/graphics/HardwareRenderer.java. Alle übrigen verwendeten Webprimärquellen sind direkt an den jeweiligen Aussagen verlinkt. Projektpins wurden in native/Cargo.lock und native/android-bridge/Cargo.toml gelesen: png 0.18.1, Tokio 1.52.3, eframe/egui 0.29.1, wgpu 22.1.0 und JNI-Bridge jni =0.22.4.

## Erstellte/geänderte Dateien

Ausschließlich /tmp/smartexplorer-login-image-review.md wurde von diesem Reviewagenten erstellt beziehungsweise fortgeschrieben. Keine Produkt-, Test-, Repositorydokumentations- oder Graphdatei wurde verändert.

