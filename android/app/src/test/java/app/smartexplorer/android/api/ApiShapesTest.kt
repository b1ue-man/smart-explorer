package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.Crumb
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.core.FilterSpec
import app.smartexplorer.android.core.Root
import app.smartexplorer.android.core.SortSpec
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.core.VolumeInfo
import kotlinx.serialization.KSerializer
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * One example of every answer form of api.md (§1–§5), decoded with the app's own DTO classes and
 * the app's own `Core.json` configuration. Answers the app decodes through private wrappers
 * (`{contactId}`, `{profileId}`, `{code}`, `{message}`, `{running}`, `{text}` of `bg.log`,
 * `{errors}` of `sync.validate`, `{rows}` of `sync.mergeRows`) are covered by the same field
 * shapes below and by the instrumented calls of the task suite.
 */
class ApiShapesTest {
    private fun <T> decode(serializer: KSerializer<T>, text: String): T = Core.json.decodeFromString(serializer, text)

    private fun <T> decodeList(serializer: KSerializer<T>, text: String): List<T> =
        Core.json.decodeFromString(ListSerializer(serializer), text)

    @Test
    fun sharedTypesDecodeWithEveryField() {
        val entry = decode(
            Entry.serializer(),
            """{"name":"Bild.jpg","location":"/storage/emulated/0/DCIM/Bild.jpg","isDir":false,"isLink":false,
               "size":2048,"mtimeMs":1727262000000,"hidden":false,"problem":"Name endet mit Punkt","kind":"image",
               "ext":"jpg","depth":2,"hasChildren":false,"expanded":false}""",
        )
        assertEquals("Bild.jpg", entry.name)
        assertEquals(2048L, entry.size)
        assertEquals("image", entry.kind)
        assertEquals(2, entry.depth)
        assertEquals("Name endet mit Punkt", entry.problem)

        val crumb = decode(Crumb.serializer(), """{"label":"DCIM","location":"/storage/emulated/0/DCIM"}""")
        assertEquals("DCIM", crumb.label)

        val root = decode(
            Root.serializer(),
            """{"id":"conn:sftp://u@h:22/","label":"Server","subtitle":null,"location":"sftp://u@h:22/","kind":"connection","removable":false}""",
        )
        assertEquals("connection", root.kind)
        assertNull(root.subtitle)

        val task = decode(
            TaskInfo.serializer(),
            """{"id":"t7","kind":"transfer","title":"Kopieren","state":"failed","doneBytes":10,"totalBytes":20,
               "doneItems":1,"totalItems":2,"rateBps":5,"message":"1 Fehler","errors":[{"path":"/a","message":"weg"}],
               "result":{"files":1,"bytes":10,"errors":1,"omitted":0},"startedMs":1,"finishedMs":2}""",
        )
        assertEquals("failed", task.state)
        assertFalse(task.isActive)
        assertEquals("/a", task.errors.single().path)
        assertEquals(1L, task.result!!.jsonObject["files"]!!.jsonPrimitive.long)
        assertEquals(2L, task.finishedMs)

        val running = decode(TaskInfo.serializer(), """{"id":"t8","kind":"scan","title":"Scan","state":"running","result":null,"finishedMs":null}""")
        assertTrue(running.isActive)

        val tasks = decodeList(TaskInfo.serializer(), """[{"id":"t1","kind":"delete","title":"Löschen","state":"queued"}]""")
        assertEquals("queued", tasks.single().state)

        val volume = decode(VolumeInfo.serializer(), """{"path":"/storage/1234-5678","label":"SD-Karte","primary":false,"removable":true}""")
        assertTrue(volume.removable)
    }

    @Test
    fun filterAndSortEncodeEveryContractField() {
        val filter = Core.json.encodeToJsonElement(FilterSpec.serializer(), FilterSpec(text = "*.txt", mode = "glob", sizeMin = 1)).jsonObject
        for (key in listOf("text", "mode", "extensions", "sizeMin", "files", "dirs", "hidden", "problemOnly")) {
            assertTrue("Filter.$key fehlt", key in filter)
        }
        // explicitNulls = false: unset optional bounds are left out instead of sent as null.
        assertFalse("sizeMax" in filter)
        assertEquals("glob", filter["mode"]!!.jsonPrimitive.content)
        val sort = Core.json.encodeToJsonElement(SortSpec.serializer(), SortSpec(key = "size", desc = true)).jsonObject
        assertEquals("size", sort["key"]!!.jsonPrimitive.content)
        assertTrue(sort["desc"]!!.jsonPrimitive.boolean)
        assertTrue(sort["dirsFirst"]!!.jsonPrimitive.boolean)
        val decoded = decode(
            FilterSpec.serializer(),
            """{"text":"a","mode":"regex","extensions":"jpg,png","sizeMin":1,"sizeMax":9,"mtimeMinMs":1,"mtimeMaxMs":2,
               "files":true,"dirs":false,"hidden":true,"problemOnly":true}""",
        )
        assertEquals(9L, decoded.sizeMax)
        assertTrue(decoded.problemOnly)
    }

    @Test
    fun systemAndPlaceAnswers() {
        val errors = decodeList(ErrorLogEntry.serializer(), """[{"timeMs":5,"action":"Kopieren","message":"Kein Platz"}]""")
        assertEquals("Kopieren", errors.single().action)
        assertEquals("Absturz", decode(TextAnswer.serializer(), """{"text":"Absturz"}""").text)
        val info = decode(CoreInfo.serializer(), """{"coreVersion":"0.5.163","dataDir":"/d/files/smart_explorer","cacheDir":"/d/cache"}""")
        assertEquals("0.5.163", info.coreVersion)
        // The init answer ({coreVersion, dataDir}) has the same field shapes.
        assertEquals("/x", decode(CoreInfo.serializer(), """{"coreVersion":"1.0.0","dataDir":"/x"}""").dataDir)

        val roots = decode(
            Roots.serializer(),
            """{"storage":[{"id":"vol:0","label":"Interner Speicher","location":"/storage/emulated/0","kind":"storage","removable":false}],
               "favorites":[{"id":"fav:1","label":"DCIM","location":"/storage/emulated/0/DCIM","kind":"favorite"}],
               "recent":[{"id":"rec:1","label":"Download","location":"/storage/emulated/0/Download","kind":"recent"}],
               "connections":[{"id":"sftp://u@h:22/","label":"u@h","location":"sftp://u@h:22/","kind":"connection"}],
               "gdrive":null,
               "devices":[{"id":"dev:1","label":"Laptop","subtitle":"Verbunden","location":"share://direct/c1","kind":"device"}],
               "rooms":[{"id":"room:1","label":"Team","location":"share://room/r1/d1","kind":"room"}],
               "trash":{"id":"trash","label":"Papierkorb","location":"trash://","kind":"trash"}}""",
        )
        assertEquals("storage", roots.storage.single().kind)
        assertNull(roots.gdrive)
        assertEquals("trash://", roots.trash!!.location)
        assertTrue(decode(FavoriteState.serializer(), """{"favorite":true}""").favorite)
    }

    @Test
    fun fileAnswers() {
        val listing = decode(
            Listing.serializer(),
            """{"location":"/storage/emulated/0","title":"Interner Speicher","crumbs":[{"label":"Interner Speicher","location":"/storage/emulated/0"}],
               "parent":null,"backend":"local","readOnly":false,"canTrash":true,
               "entries":[{"name":"DCIM","location":"/storage/emulated/0/DCIM","isDir":true,"kind":"dir"}],"totalBytes":0}""",
        )
        assertTrue(listing.isLocal)
        assertTrue(listing.canTrash)
        assertTrue(listing.entries.single().isDir)
        val zip = decode(Listing.serializer(), """{"location":"zip:///s/a.zip!/","title":"a.zip (nur lesen)","parent":"/s","backend":"zip","readOnly":true}""")
        assertTrue(zip.readOnly)

        val check = decode(NameCheck.serializer(), """{"problem":null,"exists":true,"invalid":false}""")
        assertTrue(check.exists)
        val conflicts = decode(ConflictCheck.serializer(), """{"names":["a.txt"],"choosable":true}""")
        assertEquals(listOf("a.txt"), conflicts.names)
        assertEquals("t1", decode(TaskRef.serializer(), """{"taskId":"t1"}""").taskId)
        val properties = decode(
            PropertiesResult.serializer(),
            """{"items":3,"files":2,"dirs":1,"bytes":100,"mtimeMs":5,"btimeMs":null,"location":"/s/a"}""",
        )
        assertEquals(2L, properties.files)
        assertNull(properties.btimeMs)
        assertEquals("text/plain", decode(OpenTarget.serializer(), """{"localPath":"/s/a.txt","mime":"text/plain"}""").mime)
        assertEquals("e1", decode(FetchResult.serializer(), """{"localPath":"/c/open/e1/a.txt","mime":"text/plain","editId":"e1"}""").editId)
        assertEquals(listOf("/c/share/a"), decode(MaterializeResult.serializer(), """{"paths":["/c/share/a"]}""").paths)
        val edits = decodeList(
            EditInfo.serializer(),
            """[{"editId":"e1","name":"a.txt","location":"sftp://u@h:22/a.txt","localPath":"/c/open/e1/a.txt","modified":true}]""",
        )
        assertTrue(edits.single().modified)
        assertTrue(decode(UploadConflict.serializer(), """{"conflict":true}""").conflict)
        val imported = Core.json.encodeToJsonElement(ImportFile.serializer(), ImportFile(fd = 42, name = "a.pdf", size = null)).jsonObject
        assertEquals(42L, imported["fd"]!!.jsonPrimitive.long)
        assertFalse("size" in imported)
    }

    @Test
    fun scanIndexAndTrashAnswers() {
        assertEquals("Ungültiger Ausdruck", decode(FilterCheck.serializer(), """{"error":"Ungültiger Ausdruck"}""").error)
        val view = decode(
            ScanView.serializer(),
            """{"revision":7,"unchanged":false,"entries":[{"name":"sub","location":"/s/sub","isDir":true,"depth":1,"hasChildren":true,"expanded":true}],
               "visibleTotal":12,"matches":10,"scanned":40,"truncated":false,"issues":1}""",
        )
        assertEquals(7L, view.revision)
        assertTrue(view.entries.single().hasChildren)
        assertEquals(12, view.visibleTotal)
        assertEquals("ready", decode(IndexStatus.serializer(), """{"state":"ready","count":1200}""").state)
        val hits = decodeList(FolderHit.serializer(), """[{"name":"Fotos","path":"/s/Fotos","location":"/s/Fotos","score":90}]""")
        assertEquals(90, hits.single().score)

        val trash = decodeList(
            TrashItem.serializer(),
            """[{"id":"a1","name":"b.txt","originalLocation":"/storage/1234-5678/b.txt","deletedMs":9,"size":3,"isDir":false}]""",
        )
        assertEquals("/storage/1234-5678/b.txt", trash.single().originalLocation)
        val restored = decode(RestoreResult.serializer(), """{"restored":2,"renamed":1,"failed":0,"errors":[]}""")
        assertEquals(1, restored.renamed)
        assertEquals(4, decode(PurgeResult.serializer(), """{"removed":4}""").removed)
    }

    @Test
    fun connectionAnswers() {
        val connections = decodeList(
            Connection.serializer(),
            """[{"id":"sftp://u@h:22/data","label":"u@h","protocol":"sftp","host":"h","port":22,"user":"u","root":"/data",
                "auth":"key","keyPath":"/k/id_ed25519","useAgent":false,"https":false,"location":"sftp://u@h:22/data"}]""",
        )
        assertEquals("key", connections.single().auth)
        assertEquals(22, connections.single().port)
        val input = Core.json.encodeToJsonElement(
            ConnectionInput.serializer(),
            ConnectionInput(label = "x", protocol = "ftp", host = "h", port = 21, user = "u", root = "/", auth = "password", password = "p"),
        ).jsonObject
        assertEquals("p", input["password"]!!.jsonPrimitive.content)
        assertFalse("id" in input)
        assertFalse("passphrase" in input)
        val removal = decode(EndpointRemoval.serializer(), """{"removedFavorites":2,"orphanedJobs":["Fotos sichern"]}""")
        assertEquals(listOf("Fotos sichern"), removal.orphanedJobs)
        val drive = decode(GdriveStatus.serializer(), """{"clientConfigured":true,"signedIn":false,"clientId":"abc.apps.googleusercontent.com"}""")
        assertTrue(drive.clientConfigured)
        assertFalse(drive.signedIn)
    }

    @Test
    fun syncAndBackgroundAnswers() {
        val options = decode(
            SyncOptions.serializer(),
            """{"directions":[{"value":"both","label":"Beidseitig"}],"conflicts":[{"value":"strict","label":"Nachfragen"}],
               "deletePolicies":[{"value":"propagate","label":"Übernehmen"}],"compares":[{"value":"mtimesize","label":"Zeit+Größe"}],
               "versionings":[{"value":"days","label":"Tage"}],"triggers":[{"value":"manual","label":"Manuell"}],
               "calendarKinds":[{"value":"weekly","label":"Wöchentlich"}],
               "defaults":{"id":"","name":"","source":"","target":"","direction":"both","conflict":"strict","deletePolicy":"propagate",
                           "compare":"mtimesize","versioning":"days","retainDays":30,"trigger":"manual","intervalMin":60,"calendar":null,
                           "rtDebounceSecs":5,"includeHidden":false,"ignore":[],"enabled":true,"runBefore":"","runAfter":"","lastRun":0,
                           "activeFromMin":0,"activeToMin":0,"catchUp":false,"moveFiles":false,"maxDelete":0,"maxDeletePct":0,
                           "useRecycleBin":true,"lastResult":null,"schedule":"Manuell","runningTask":null}}""",
        )
        assertEquals("weekly", options.calendarKinds.single().value)
        assertEquals(30, options.defaults.retainDays)
        val jobs = decodeList(
            SyncJob.serializer(),
            """[{"id":"j1","name":"Fotos","source":"/s/DCIM","target":"sftp://u@h:22/fotos","direction":"a2b","conflict":"newer",
                "deletePolicy":"nodelete","compare":"mtimesize","versioning":"days","retainDays":7,"trigger":"calendar","intervalMin":60,
                "calendar":{"kind":"weekly","minuteOfDay":450,"weekday":5,"monthday":1},"rtDebounceSecs":5,"includeHidden":true,
                "ignore":["*.tmp"],"enabled":true,"runBefore":"","runAfter":"","lastRun":1727262000,"activeFromMin":0,"activeToMin":0,
                "catchUp":true,"moveFiles":false,"maxDelete":10,"maxDeletePct":50,"useRecycleBin":true,
                "lastResult":{"timeMs":1727262000000,"aToB":3,"bToA":0,"deleted":1,"conflicts":0,"errors":0,"note":"ok"},
                "schedule":"Mo, Mi 07:30","runningTask":"t9"}]""",
        )
        val job = jobs.single()
        assertEquals(5, job.calendar!!.weekday)
        assertEquals(3, job.lastResult!!.aToB)
        assertEquals("t9", job.runningTask)
        val run = decode(
            SyncRunResult.serializer(),
            """{"summary":"3 →, 0 ←","aToB":3,"bToA":0,"deleted":0,"conflicts":1,"errors":0,"omitted":"1 geschützte Auslassung"}""",
        )
        assertEquals(1, run.conflicts)
        val mirror = decode(MirrorResult.serializer(), """{"summary":"fertig","copied":4,"skipped":1,"errors":0,"omitted":null}""")
        assertEquals(4L, mirror.copied)
        val numeric = decode(
            SyncConflicts.serializer(),
            """{"available":true,"items":[{"cid":3,"path":"a.txt","a":{"exists":true,"size":1,"mtimeMs":2},"b":{"exists":false,"size":0,"mtimeMs":0},"text":true}]}""",
        )
        assertEquals(JsonPrimitive(3), numeric.items.single().cid)
        val textual = decode(SyncConflicts.serializer(), """{"available":false,"items":[{"cid":"c-1","path":"b.txt"}]}""")
        assertEquals("\"c-1\"", textual.items.single().key)
        val rows = Core.json.decodeFromJsonElement(
            ListSerializer(MergeRow.serializer()),
            Core.json.parseToJsonElement("""{"rows":[{"a":"x","b":null,"equal":false,"takeA":true,"takeB":false}]}""").jsonObject["rows"]!!,
        )
        assertNull(rows.single().b)
        val status = decode(
            BgStatus.serializer(),
            """{"syncEnabled":true,"daemonRunning":true,"heartbeatAgeSecs":3,"paused":true,"pausedUntilMs":null,"autopauseBattery":true,
               "autopauseMetered":false,"cadenceSecs":30,"catchUpRunning":false,"lastCatchUpMs":1727262000000,"activeJob":"Fotos"}""",
        )
        assertTrue(status.paused)
        assertNull(status.pausedUntilMs)
        assertEquals("Fotos", status.activeJob)
        val catchUp = decode(
            CatchUpResult.serializer(),
            """{"admitted":1,"skipped":[{"jobId":"j2","jobName":"Musik","reason":"kürzlich versucht"}],"message":"1 Job ausgeführt"}""",
        )
        assertEquals("Musik", catchUp.skipped.single().jobName)
    }

    @Test
    fun analysisAndUpdateAnswers() {
        val node = decode(
            AnalyzeNode.serializer(),
            """{"name":"0","size":300,"isDir":true,"children":[{"name":"big.bin","size":200,"isDir":false,"childCount":0},
               {"name":"sub","size":100,"isDir":true,"childCount":2}],"location":"/storage/emulated/0"}""",
        )
        assertEquals("big.bin", node.children.first().name)
        assertEquals(2, node.children.last().childCount)
        assertEquals(1, decode(AnalyzeIssues.serializer(), """{"count":1,"text":"Kein Zugriff"}""").count)
        val groups = decodeList(
            DuplicateGroup.serializer(),
            """[{"size":50,"items":[{"location":"/s/a.dat","mtimeMs":1},{"location":"/s/sub/b.dat","mtimeMs":2}]}]""",
        )
        assertEquals(2, groups.single().items.size)
        val update = decode(UpdateInfo.serializer(), """{"current":"0.5.162","latest":"0.5.163","available":true,"notes":null}""")
        assertTrue(update.available)
        val download = decode(UpdateDownload.serializer(), """{"path":"/c/update/smart-explorer-android-0.5.163.apk","version":"0.5.163"}""")
        assertEquals("0.5.163", download.version)
    }

    @Test
    fun shareAnswers() {
        val status = decode(
            ShareStatus.serializer(),
            """{"running":true,"connected":true,"relayUrl":"http://10.0.0.4:51821","lastError":null,"server":"10.0.0.4:51820",
               "lanPresence":"aus","identity":{"deviceId":"d1","deviceName":"Telefon","fingerprint":"AB:CD","directCode":"SE-D3-x"},
               "devices":[{"contactId":"c1","name":"Laptop","status":"connectedRelay","statusText":"Verbunden (Relay)","online":true,
                           "location":"share://direct/c1","lan":false}],
               "rooms":[{"profileId":"p1","roomId":"r1","name":"Team","status":"Verbunden","autoJoin":true,"location":null,
                         "members":[{"deviceId":"d2","name":"Laptop","status":"Verbunden","location":"share://room/p1/d2","blocked":false}]}],
               "incoming":[{"requestId":"q1","contactId":"c1","name":"Laptop","stateText":"Offen","canAccept":true,"canReject":true,
                            "canRetry":false,"canDelete":true,"message":null,"timeMs":5}],
               "outgoing":[],
               "exports":{"direct":[{"label":"Home","path":"/storage/emulated/0"}],"rooms":{"p1":[{"label":"Docs","path":"/storage/emulated/0/Docs"}]}},
               "discovery":{"offer":{"offerId":"o1","target":"direct","alias":"Telefon","untilMs":9},
                            "advertisements":[{"discoveryId":"a1","kind":"room","alias":"Team","expiresMs":9,"compatible":true}],
                            "exchange":{"exchangeId":"x1","state":"running","message":null}},
               "removedDevices":[{"deviceId":"d3","name":"Alt"}],"notices":["Share-Server fehlt"]}""",
        )
        assertTrue(status.connected)
        assertEquals("share://room/p1/d2", status.rooms.single().members.single().location)
        assertEquals("Docs", status.exports.rooms.getValue("p1").single().label)
        assertTrue(status.incoming.single().canAccept)
        assertEquals("o1", status.discovery.offer!!.offerId)
        assertEquals("room", status.discovery.advertisements.single().kind)
        assertEquals("d3", status.removedDevices.single().deviceId)
        val empty = decode(ShareStatus.serializer(), """{"running":false,"connected":false,"server":null,"identity":{},"discovery":{}}""")
        assertNull(empty.server)
        assertTrue(empty.rooms.isEmpty())
        val created = decode(RoomCreated.serializer(), """{"profileId":"p2","code":"SE-R3-abc-def"}""")
        assertTrue(created.code.startsWith("SE-R3-"))
        val exec = decode(ExecResult.serializer(), """{"stdout":"ok\n","stderr":"","exitCode":0,"timedOut":false,"truncated":false}""")
        assertEquals(0, exec.exitCode)
        assertEquals(1, decode(EndpointRemoval.serializer(), """{"removedFavorites":1,"orphanedJobs":[]}""").removedFavorites)
        // Answers the app reads through private wrappers: the same shapes as plain objects.
        for (text in listOf("""{"contactId":"c9"}""", """{"profileId":"p9"}""", """{"code":null}""", """{"message":"Verbindung OK"}""")) {
            assertEquals(1, Core.json.parseToJsonElement(text).jsonObject.size)
        }
        assertTrue(JsonObject(emptyMap()).isEmpty())
    }
}
