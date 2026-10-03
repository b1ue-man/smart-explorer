package app.smartexplorer.android.task

import android.util.Base64
import java.io.ByteArrayOutputStream
import java.io.Closeable
import java.io.InputStream
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketException
import java.net.SocketTimeoutException
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.serialization.json.JsonObject

/** Real, isolated FTP: MLST/MLSD, streaming passive transfers and one refused stage RNTO. */
internal class ReviewMergeFtpFixture(port: Int = 0, snapshot: JsonObject? = null, username: String = "review") : Closeable {
    private data class Node(val bytes: ByteArray?, var modified: Long)
    private val loopback = InetAddress.getByAddress(byteArrayOf(127, 0, 0, 1))
    private val guard = Any()
    private val nodes = linkedMapOf<String, Node>()
    private val uploads = mutableSetOf<String>()
    private val sockets = ConcurrentHashMap.newKeySet<Socket>()
    private val listeners = ConcurrentHashMap.newKeySet<ServerSocket>()
    private val faults = ConcurrentLinkedQueue<String>()
    private val closed = AtomicBoolean(false)
    private val workers = ThreadPoolExecutor(12, 12, 0, TimeUnit.MILLISECONDS,
        ArrayBlockingQueue<Runnable>(16))
    private val control: ServerSocket
    val port: Int get() = control.localPort
    val root = "/review-sync"
    val user = snapshot?.text("user") ?: username
    private var refusal: Pair<String, ByteArray>? = null
    private var rejected = 0
    val rejectedPublishes: Int get() = synchronized(guard) { rejected }

    init {
        if (snapshot == null) {
            nodes["/"] = Node(null, now())
            nodes[root] = Node(null, now())
        } else {
            require(snapshot.text("root") == root) { "FTP root changed across restart" }
            snapshot.objects("nodes").forEach { row ->
                val path = resolve("/", row.text("path"))
                require(nodes.size < 512)
                require(nodes.put(path, Node(row.textOrNull("bytes")?.let(::decode), row.long("modified"))) == null)
            }
            require(nodes.containsKey("/") && nodes["/"]?.bytes == null && nodes.containsKey(root) && nodes[root]?.bytes == null)
        }
        control = listen(port)
    }

    private val acceptor = Thread({
        while (!closed.get()) {
            try {
                val socket = control.accept()
                sockets.add(socket)
                try { socket.soTimeout = 30_000; workers.execute { serve(socket) } }
                catch (e: Exception) {
                    try { if (e is RuntimeException && !closed.get()) reply(socket, "421 Fixture connection limit") }
                    finally { socket.close(); sockets.remove(socket) }
                    if (e !is RuntimeException) throw e
                }
            } catch (_: SocketTimeoutException) {
                // Recheck close without an unbounded accept.
            } catch (e: Exception) {
                if (!closed.get()) faults.add("accept: $e")
                break
            }
        }
    }, "review-ftp-accept").apply { isDaemon = true; start() }

    fun write(name: String, text: String) = synchronized(guard) {
        val path = resolve(root, name)
        require(parent(path) == root)
        nodes[path] = Node(text.toByteArray(Charsets.UTF_8), now())
    }

    fun bytes(name: String): ByteArray = synchronized(guard) {
        nodes[resolve(root, name)]?.bytes?.copyOf() ?: error("FTP file missing: $name")
    }

    fun refuseNextPublication(name: String, expected: ByteArray) = synchronized(guard) {
        require(refusal == null)
        refusal = resolve(root, name) to expected.copyOf()
    }

    fun snapshot(): JsonObject = synchronized(guard) {
        args("root" to root, "user" to user, "nodes" to nodes.map { (path, node) ->
            mapOf("path" to path, "bytes" to node.bytes?.let(::encode), "modified" to node.modified)
        })
    }

    fun assertHealthy() { check(faults.isEmpty()) { "FTP fixture failed: $faults" } }

    private fun listen(port: Int): ServerSocket {
        val listener = ServerSocket()
        try {
            listener.reuseAddress = true
            listener.bind(InetSocketAddress(loopback, port), 16)
            listener.soTimeout = 1_000
            listeners.add(listener)
            return listener
        } catch (e: Throwable) {
            listener.close()
            throw e
        }
    }

    private fun serve(socket: Socket) {
        var passive: ServerSocket? = null
        var cwd = "/"
        var from: String? = null
        var rest = 0
        var named = false
        var loggedIn = false
        try {
            reply(socket, "220 Review FTP ready")
            val input = socket.getInputStream()
            while (!closed.get()) {
                val line = line(input) ?: break
                val command = line.substringBefore(' ').uppercase(Locale.ROOT)
                val argument = line.substringAfter(' ', "")
                fun path() = resolve(cwd, argument)
                if (!loggedIn && command !in setOf("USER", "PASS", "FEAT", "QUIT")) {
                    reply(socket, "530 Login required"); continue
                }
                when (command) {
                    "USER" -> { named = argument == user; loggedIn = false; reply(socket, if (named) "331 Password required" else "530 Unknown user") }
                    "PASS" -> { loggedIn = named && argument == PASSWORD; reply(socket, if (loggedIn) "230 Logged in" else "530 Login incorrect") }
                    "TYPE", "OPTS", "NOOP" -> reply(socket, "200 OK")
                    "SYST" -> reply(socket, "215 UNIX Type: L8")
                    "FEAT" -> reply(socket, "211-Features\r\n MLST type*;size*;modify*;\r\n MFMT\r\n REST STREAM\r\n UTF8\r\n211 End")
                    "PWD" -> reply(socket, "257 \"$cwd\" is current directory")
                    "CWD" -> {
                        val destination = path()
                        if (isDir(destination)) { cwd = destination; reply(socket, "250 Directory changed") }
                        else reply(socket, "550 No such directory")
                    }
                    "PASV", "EPSV" -> {
                        passive?.let(::closeListener)
                        val opened = listen(0).apply { soTimeout = 5_000 }
                        passive = opened
                        if (command == "EPSV") reply(socket, "229 Entering Extended Passive Mode (|||${opened.localPort}|)")
                        else reply(socket, "227 Entering Passive Mode (127,0,0,1,${opened.localPort / 256},${opened.localPort % 256})")
                    }
                    "MLST" -> {
                        val destination = path()
                        val facts = synchronized(guard) { nodes[destination]?.let { facts(it, destination) } }
                        if (facts == null) reply(socket, "550 No such entry")
                        else reply(socket, "250-Listing\r\n $facts\r\n250 End")
                    }
                    "MLSD", "LIST" -> {
                        val folder = if (argument.isEmpty() || argument == "-a") cwd else path()
                        if (!isDir(folder)) reply(socket, "550 No such directory")
                        else {
                            val payload = synchronized(guard) {
                                nodes.filter { (name, _) -> name != folder && parent(name) == folder }
                                    .map { (name, node) -> if (command == "MLSD") facts(node, name.substringAfterLast('/'))
                                        else "${if (node.bytes == null) "d" else "-"}rwxr-xr-x 1 0 0 ${node.bytes?.size ?: 0} Jan 1 2020 ${name.substringAfterLast('/')}" }
                                    .joinToString("\r\n", postfix = "\r\n").toByteArray(Charsets.UTF_8)
                            }
                            transfer(socket, passive) { it.getOutputStream().write(payload) }
                            passive = null
                        }
                    }
                    "SIZE", "MDTM" -> {
                        val value = synchronized(guard) { nodes[path()]?.takeIf { it.bytes != null }?.let {
                            if (command == "SIZE") it.bytes!!.size.toString() else stamp(it.modified)
                        } }
                        reply(socket, if (value == null) "550 No readable regular file" else "213 $value")
                    }
                    "MFMT" -> {
                        val time = argument.substringBefore(' ')
                        val destination = resolve(cwd, argument.substringAfter(' ', ""))
                        val applied = synchronized(guard) { nodes[destination]?.let { it.modified = parseStamp(time); true } ?: false }
                        reply(socket, if (applied) "213 Modify=$time; $destination" else "550 No such file")
                    }
                    "REST" -> { rest = argument.toInt(); require(rest >= 0); reply(socket, "350 Restart accepted") }
                    "RETR" -> {
                        val bytes = synchronized(guard) { nodes[path()]?.bytes?.copyOf() }
                        if (bytes == null) reply(socket, "550 No such file")
                        else {
                            val offset = rest.coerceAtMost(bytes.size); rest = 0
                            transfer(socket, passive) { it.getOutputStream().write(bytes, offset, bytes.size - offset) }
                            passive = null
                        }
                    }
                    "STOR" -> {
                        val destination = path()
                        if (!isDir(parent(destination))) reply(socket, "550 Parent directory missing")
                        else {
                            transfer(socket, passive) { data ->
                                val body = body(data.getInputStream())
                                synchronized(guard) {
                                    require(nodes[destination]?.bytes != null || !nodes.containsKey(destination))
                                    require(nodes.size < 512 || nodes.containsKey(destination))
                                    nodes[destination] = Node(body, now()); uploads.add(destination)
                                }
                            }
                            passive = null
                        }
                    }
                    "RNFR" -> {
                        val source = path()
                        if (synchronized(guard) { nodes.containsKey(source) }) { from = source; reply(socket, "350 Ready for RNTO") }
                        else reply(socket, "550 Source missing")
                    }
                    "RNTO" -> {
                        val source = from; from = null
                        val destination = path()
                        val outcome = synchronized(guard) {
                            val node = source?.let(nodes::get)
                            when {
                                source == null || node == null || node.bytes == null || !isDir(parent(destination)) -> "550 Rename unavailable"
                                refusal?.let { (target, expected) -> destination == target && source != destination
                                    && source in uploads && node.bytes.contentEquals(expected) } == true -> {
                                    refusal = null; rejected++; "550 Review fixture refused this stage publication"
                                }
                                nodes[destination]?.bytes == null && nodes.containsKey(destination) -> "550 Target is directory"
                                else -> { nodes.remove(source); uploads.remove(source); nodes[destination] = node; "250 Rename complete" }
                            }
                        }
                        reply(socket, outcome)
                    }
                    "MKD" -> {
                        val destination = path()
                        val made = synchronized(guard) {
                            if (nodes.containsKey(destination) || !isDir(parent(destination))) false
                            else { require(nodes.size < 512); nodes[destination] = Node(null, now()); true }
                        }
                        reply(socket, if (made) "257 \"$destination\" created" else "550 Directory unavailable")
                    }
                    "DELE", "RMD" -> {
                        val destination = path()
                        val removed = synchronized(guard) {
                            val node = nodes[destination]
                            val valid = node != null && if (command == "DELE") node.bytes != null
                                else node.bytes == null && nodes.keys.none { it != destination && parent(it) == destination }
                            if (valid && destination != "/" && destination != root) { nodes.remove(destination); uploads.remove(destination); true } else false
                        }
                        reply(socket, if (removed) "250 Removed" else "550 Entry unavailable")
                    }
                    "ABOR" -> reply(socket, "226 Abort complete")
                    "QUIT" -> { reply(socket, "221 Bye"); break }
                    else -> reply(socket, "502 Command not implemented")
                }
            }
        } catch (_: SocketTimeoutException) {
            // Idle control connections may expire; the real client reconnects them.
        } catch (e: SocketException) {
            if (!closed.get() && !socket.isClosed && e.message?.contains("reset", ignoreCase = true) != true) faults.add("control: $e")
        } catch (e: Exception) {
            if (!closed.get()) faults.add("control: $e")
        } finally {
            passive?.let(::closeListener)
            sockets.remove(socket); socket.close()
        }
    }

    private fun transfer(control: Socket, passive: ServerSocket?, work: (Socket) -> Unit) {
        if (passive == null) { reply(control, "425 Use passive mode first"); return }
        try {
            reply(control, "150 Opening data connection")
            val data = passive.accept().apply { soTimeout = 5_000 }
            sockets.add(data)
            try { data.use(work) } finally { sockets.remove(data) }
            reply(control, "226 Transfer complete")
        } finally { closeListener(passive) }
    }

    private fun isDir(path: String): Boolean = synchronized(guard) { nodes.containsKey(path) && nodes[path]?.bytes == null }
    private fun facts(node: Node, name: String) = "type=${if (node.bytes == null) "dir" else "file"};size=${node.bytes?.size ?: 0};modify=${stamp(node.modified)}; $name"
    private fun parent(path: String) = path.substringBeforeLast('/', "").ifEmpty { "/" }
    private fun resolve(cwd: String, input: String): String {
        require(input.none { it == '\r' || it == '\n' || it == '\u0000' })
        val path = if (input.startsWith('/')) input else "$cwd/$input"
        val parts = path.split('/').filter { it.isNotEmpty() }
        require(parts.none { it == "." || it == ".." })
        val resolved = "/" + parts.joinToString("/")
        require(resolved == "/" || resolved == root || resolved.startsWith("$root/"))
        return resolved
    }
    private fun line(input: InputStream): String? {
        val bytes = ByteArrayOutputStream()
        while (true) {
            val next = input.read()
            if (next < 0) { require(bytes.size() == 0); return null }
            if (next == 10) return bytes.toString("UTF-8").removeSuffix("\r")
            require(bytes.size() < 8192) { "FTP command too long" }; bytes.write(next)
        }
    }
    private fun body(input: InputStream): ByteArray {
        val bytes = ByteArrayOutputStream(); val buffer = ByteArray(8192)
        while (true) {
            val count = input.read(buffer)
            if (count < 0) return bytes.toByteArray()
            require(bytes.size() + count <= 8 * 1024 * 1024) { "FTP fixture body too large" }
            bytes.write(buffer, 0, count)
        }
    }
    private fun reply(socket: Socket, value: String) { socket.getOutputStream().apply { write((value + "\r\n").toByteArray(Charsets.UTF_8)); flush() } }
    private fun closeListener(listener: ServerSocket) { listeners.remove(listener); listener.close() }
    private fun now() = System.currentTimeMillis() / 1000 * 1000
    private fun format() = SimpleDateFormat("yyyyMMddHHmmss", Locale.US).apply { timeZone = TimeZone.getTimeZone("UTC"); isLenient = false }
    private fun stamp(time: Long) = format().format(Date(time))
    private fun parseStamp(value: String): Long { require(value.length == 14); return requireNotNull(format().parse(value)).time }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        listeners.toList().forEach { runCatching { it.close() } }
        sockets.toList().forEach { runCatching { it.close() } }
        workers.shutdownNow()
        acceptor.join(2_000)
        val stopped = workers.awaitTermination(5, TimeUnit.SECONDS)
        check(!acceptor.isAlive && stopped) { "FTP fixture threads did not stop" }
    }

    companion object {
        const val PASSWORD = "review-fixture"
        fun encode(bytes: ByteArray): String = Base64.encodeToString(bytes, Base64.NO_WRAP)
        fun decode(text: String): ByteArray = Base64.decode(text, Base64.DEFAULT)
    }
}
