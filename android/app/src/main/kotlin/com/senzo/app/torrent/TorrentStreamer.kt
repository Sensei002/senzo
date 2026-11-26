package com.senzo.app.torrent

import org.libtorrent4j.FileStorage
import org.libtorrent4j.Priority
import org.libtorrent4j.SessionManager
import org.libtorrent4j.TorrentFlags
import org.libtorrent4j.TorrentHandle
import org.libtorrent4j.TorrentInfo
import java.io.BufferedInputStream
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import java.io.RandomAccessFile
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.Locale

/**
 * Plays torrents by downloading them on the device and serving the chosen file
 * over a loopback HTTP address, which is what the player already knows.
 *
 * The desktop client runs a torrent server too. The behaviour is the same: only
 * the file the viewer asked for is prioritised, pieces are requested in order,
 * and the player can seek because the loopback server answers range requests.
 */
class TorrentStreamer(private val cacheDir: File) {

    /** What the viewer sees while the torrent connects. */
    data class Status(
        val name: String = "",
        val percent: Int = 0,
        val seeds: Int = 0,
        val peers: Int = 0,
        val rate: Long = 0,
        val state: String = "starting",
        val error: String? = null,
    ) {
        val label: String
            get() = when {
                error != null -> error
                state == "metadata" -> if (peers + seeds > 0) {
                    "Asking ${peers + seeds} peers for the file…"
                } else {
                    "Looking for peers…"
                }
                state == "finished" -> "Ready"
                percent > 0 -> "$percent% of the file · $peers peers"
                else -> "Connecting…"
            }
    }

    private val lock = Any()
    private var session: SessionManager? = null
    @Volatile private var handle: TorrentHandle? = null
    @Volatile private var file: RandomAccessFile? = null
    private var server: ServerSocket? = null
    @Volatile private var pieceLength = 1
    @Volatile private var fileSize = 0L
    @Volatile private var fileOffset = 0L
    @Volatile private var videoName = ""
    private var videoIndex = -1
    @Volatile private var current = Status()
    @Volatile private var stopped = true

    val status: Status get() = current

    /**
     * Starts [magnet] and returns the loopback address the player opens.
     * Blocks while the metadata and the first pieces arrive, so call it off the
     * main thread.
     */
    fun start(magnet: String): String {
        // Let a previous run leave its waits before taking the lock, otherwise
        // it would sit in a 90 s wait while this call blocks behind it.
        stopped = true
        synchronized(lock) {
            stopLocked()
            stopped = false
            val dir = File(cacheDir, "stream").apply { mkdirs() }
            current = Status(state = "metadata")
            val session = try {
                SessionManager().also { it.start() }
            } catch (error: Throwable) {
                stopped = true
                throw IllegalStateException("The torrent engine is not available on this device.", error)
            }
            this.session = session
            val torrent = try {
                resolveTorrent(session, magnet, dir)
            } catch (error: Throwable) {
                safeSessionStop(session)
                this.session = null
                stopped = true
                current = current.copy(error = error.message ?: "The torrent could not be resolved.")
                throw error
            }
            val storage = torrent.files()
            val index = pickVideoFile(storage)
            if (index < 0) {
                safeSessionStop(session)
                this.session = null
                stopped = true
                val message = "This torrent has no playable video file."
                current = current.copy(error = message)
                throw IllegalStateException(message)
            }
            videoIndex = index
            pieceLength = torrent.pieceLength().coerceAtLeast(1)
            fileOffset = storage.fileOffset(index)
            fileSize = storage.fileSize(index)
            videoName = storage.filePath(index)
            session.download(torrent, dir)
            val added = waitForHandle(session, torrent)
                ?: throw IllegalStateException("The torrent could not be added.")
            handle = added
            prioritize(added, storage)
            // Wait for the opening pieces here, so the player connects to a
            // server that can answer at once instead of timing out.
            val first = (fileOffset / pieceLength).toInt()
            waitForPieces(added, first, PREBUFFER_SECONDS)
            // Where libtorrent put the file depends on how the torrent is
            // laid out, so ask the disk rather than guessing.
            val path = locate(dir, storage.filePath(index), torrent.name())
            path.parentFile?.mkdirs()
            file = RandomAccessFile(path, "rw")
            val socket = try {
                ServerSocket(0, 16, InetAddress.getByName("127.0.0.1"))
            } catch (error: Throwable) {
                stopLocked()
                throw IllegalStateException("The local stream server could not start.", error)
            }
            server = socket
            Thread({ acceptLoop(socket) }, "senzo-torrent-http").apply { isDaemon = true }.start()
            Thread({ watch(added) }, "senzo-torrent-progress").apply { isDaemon = true }.start()
            return "http://127.0.0.1:${socket.localPort}/stream"
        }
    }

    /** Stops the stream and releases the torrent. Safe to call twice. */
    fun stop() {
        stopped = true
        synchronized(lock) { stopLocked() }
    }

    private fun stopLocked() {
        stopped = true
        runCatching { server?.close() }
        server = null
        runCatching { file?.close() }
        file = null
        runCatching { handle?.let { currentHandle -> session?.remove(currentHandle) } }
        handle = null
        session?.let { safeSessionStop(it) }
        session = null
        videoIndex = -1
        videoName = ""
        fileSize = 0
        fileOffset = 0
        if (current.error == null) current = Status()
    }

    private fun safeSessionStop(session: SessionManager) {
        runCatching { session.stop() }
    }

    private fun waitForHandle(session: SessionManager, torrent: TorrentInfo): TorrentHandle? {
        val deadline = System.currentTimeMillis() + 15_000
        while (System.currentTimeMillis() < deadline && !stopped) {
            val found = runCatching { session.find(torrent.infoHash()) }.getOrNull()
            if (found != null) return found
            Thread.sleep(100)
        }
        return null
    }

    /**
     * A magnet without trackers relies on DHT alone, which can take a while on
     * mobile networks. The public trackers below give it an immediate swarm to
     * ask, and are only added when the magnet carries none of its own.
     */
    private fun withTrackers(magnet: String): String {
        if (magnet.contains("tr=")) return magnet
        val separator = if (magnet.contains('?')) '&' else '?'
        val extras = TRACKERS.joinToString("") { tracker ->
            separator.toString() + "tr=" + java.net.URLEncoder.encode(tracker, "UTF-8")
        }
        return magnet + extras
    }

    /** Finds the file libtorrent wrote, whether or not it nested it. */
    private fun locate(saveDir: File, relative: String, torrentName: String): File {
        val direct = File(saveDir, relative)
        if (direct.exists()) return direct
        val nested = File(File(saveDir, torrentName), relative)
        if (nested.exists()) return nested
        return direct
    }

    /** Blocks until [count] pieces starting at [first] are on disk. */
    private fun waitForPieces(torrent: TorrentHandle, first: Int, timeoutSeconds: Int) {
        val deadline = System.currentTimeMillis() + timeoutSeconds * 1_000L
        while (!stopped && System.currentTimeMillis() < deadline) {
            if (torrent.havePiece(first)) return
            val status = runCatching { torrent.status() }.getOrNull()
            if (status != null) {
                current = Status(
                    name = status.name() ?: videoName,
                    percent = (status.progress() * 100).toInt(),
                    seeds = status.numSeeds(),
                    peers = status.numPeers(),
                    rate = status.downloadRate().toLong(),
                    state = "downloading",
                )
            }
            runCatching { torrent.setPieceDeadline(first, 900) }
            Thread.sleep(200)
        }
        if (stopped) throw IllegalStateException("The torrent was stopped.")
        val peers = current.peers
        val message = if (peers == 0) {
            "No peers are sharing this torrent right now. Try another source."
        } else {
            "The swarm is not sending data yet ($peers peers). Try another source."
        }
        throw IllegalStateException(message)
    }

    /**
     * Resolves a magnet to torrent metadata.
     *
     * Adding the magnet as a normal download is the standard flow: the engine
     * connects, trades metadata, and starts on the opening pieces at once.
     * `fetchMagnet` runs in upload mode and drops the torrent when it gives up,
     * so it is only the fallback for swarms that answer the second way.
     */
    private fun resolveTorrent(session: SessionManager, magnet: String, dir: File): TorrentInfo {
        val uri = withTrackers(magnet)
        val hash = hashOf(magnet)
        if (hash != null) {
            session.download(uri, dir, org.libtorrent4j.swig.torrent_flags_t())
            val deadline = System.currentTimeMillis() + METADATA_SECONDS * 1_000L
            while (!stopped && System.currentTimeMillis() < deadline) {
                val handle = runCatching { session.find(hash) }.getOrNull()
                if (handle != null) {
                    val info = runCatching { handle.torrentFile() }.getOrNull()
                    if (info != null && info.isValid()) return info
                    val status = runCatching { handle.status() }.getOrNull()
                    if (status != null) {
                        current = current.copy(
                            state = "metadata",
                            seeds = status.numSeeds(),
                            peers = status.numPeers(),
                        )
                    }
                }
                Thread.sleep(400)
            }
            if (stopped) throw IllegalStateException("The torrent was stopped.")
        }
        val raw = runCatching {
            session.fetchMagnet(uri, 30, File(dir, "metadata.torrent"))
        }.getOrNull() ?: throw IllegalStateException("No peer answered with the torrent metadata.")
        return TorrentInfo(raw)
    }

    /** Reads the info hash out of a magnet so the added torrent can be found. */
    private fun hashOf(magnet: String): org.libtorrent4j.Sha1Hash? {
        val raw = Regex("urn:btih:([a-zA-Z0-9]+)").find(magnet)?.groupValues?.getOrNull(1)
            ?: return null
        if (raw.length != 40) return null
        return runCatching { org.libtorrent4j.Sha1Hash.parseHex(raw.lowercase()) }.getOrNull()
    }

    private fun prioritize(torrent: TorrentHandle, storage: FileStorage) {
        runCatching { torrent.setFlags(TorrentFlags.SEQUENTIAL_DOWNLOAD, TorrentFlags.SEQUENTIAL_DOWNLOAD) }
        val priorities = Array(storage.numFiles()) { Priority.IGNORE }
        if (videoIndex in priorities.indices) priorities[videoIndex] = Priority.TOP_PRIORITY
        runCatching { torrent.prioritizeFiles(priorities) }
        val first = (fileOffset / pieceLength).toInt()
        val last = ((fileOffset + fileSize - 1) / pieceLength).toInt()
        runCatching { torrent.setSequentialRange(first, last) }
    }

    /** Updates the label the catalog shows while the torrent is open. */
    private fun watch(torrent: TorrentHandle) {
        while (!stopped) {
            val status = runCatching { torrent.status() }.getOrNull() ?: return
            current = Status(
                name = status.name() ?: "",
                percent = (status.progress() * 100).toInt(),
                seeds = status.numSeeds(),
                peers = status.numPeers(),
                rate = status.downloadRate().toLong(),
                state = when {
                    status.isFinished() -> "finished"
                    !status.hasMetadata() -> "metadata"
                    status.isSeeding() -> "finished"
                    else -> "downloading"
                },
            )
            Thread.sleep(1_000)
        }
    }

    private fun acceptLoop(socket: ServerSocket) {
        while (!stopped) {
            val client = try {
                socket.accept()
            } catch (error: Throwable) {
                return
            }
            Thread({
                runCatching { serve(client) }
                runCatching { client.close() }
            }, "senzo-torrent-client").apply { isDaemon = true }.start()
        }
    }

    private fun serve(client: Socket) {
        val input = BufferedInputStream(client.getInputStream())
        val output = client.getOutputStream()
        val request = readLine(input) ?: return
        val parts = request.split(" ")
        if (parts.size < 2) return
        val method = parts[0]
        var start = 0L
        var end = fileSize - 1
        var partial = false
        while (true) {
            val line = readLine(input) ?: break
            if (line.isEmpty()) break
            if (!line.lowercase(Locale.US).startsWith("range:")) continue
            val value = line.substringAfter("=").trim()
            val range = value.removePrefix("bytes=")
            val left = range.substringBefore("-").trim()
            val right = range.substringAfter("-", "").trim()
            if (left.isEmpty()) {
                val count = right.toLongOrNull() ?: continue
                start = (fileSize - count).coerceAtLeast(0)
            } else {
                start = left.toLongOrNull() ?: continue
            }
            if (right.isNotEmpty() && left.isNotEmpty()) {
                end = right.toLongOrNull()?.coerceAtMost(fileSize - 1) ?: end
            }
            partial = true
        }
        if (fileSize <= 0) {
            writeStatus(output, 503, "Service Unavailable", "The torrent has no file yet.")
            return
        }
        start = start.coerceIn(0, fileSize - 1)
        end = end.coerceIn(start, fileSize - 1)
        val length = end - start + 1
        val header = buildString {
            append(if (partial) "HTTP/1.1 206 Partial Content\r\n" else "HTTP/1.1 200 OK\r\n")
            append("Accept-Ranges: bytes\r\n")
            append("Content-Type: ").append(mimeFor(videoName)).append("\r\n")
            append("Content-Length: ").append(length).append("\r\n")
            if (partial) {
                append("Content-Range: bytes ").append(start).append("-").append(end)
                append("/").append(fileSize).append("\r\n")
            }
            append("Connection: close\r\n\r\n")
        }
        output.write(header.toByteArray())
        if (method.equals("HEAD", ignoreCase = true)) {
            output.flush()
            return
        }
        val buffer = ByteArray(64 * 1024)
        var position = start
        while (position <= end) {
            val want = minOf(buffer.size.toLong(), end - position + 1).toInt()
            if (!awaitRange(position, position + want - 1)) return
            val read = readAt(position, buffer, want)
            if (read <= 0) return
            output.write(buffer, 0, read)
            position += read
        }
        output.flush()
    }

    private fun readAt(offset: Long, buffer: ByteArray, length: Int): Int {
        val target = file ?: return -1
        synchronized(target) {
            target.seek(fileOffset + offset)
            return target.read(buffer, 0, length)
        }
    }

    /**
     * Blocks until every piece covering the byte range has arrived. The pieces
     * just ahead of the viewer are given deadlines, which is what turns a
     * sequential download into a stream that starts in seconds.
     */
    private fun awaitRange(start: Long, end: Long): Boolean {
        val torrent = handle ?: return false
        val first = (start / pieceLength).toInt()
        val last = (end / pieceLength).toInt()
        val deadline = System.currentTimeMillis() + RANGE_SECONDS * 1_000
        while (!stopped && System.currentTimeMillis() < deadline) {
            if (piecesReady(torrent, first, last)) return true
            for (piece in first..minOf(last, first + 48)) {
                runCatching { torrent.setPieceDeadline(piece, 800) }
            }
            Thread.sleep(120)
        }
        return false
    }

    private fun piecesReady(torrent: TorrentHandle, first: Int, last: Int): Boolean {
        for (piece in first..last) {
            val present = runCatching { torrent.havePiece(piece) }.getOrDefault(false)
            if (!present) return false
        }
        return true
    }

    private fun writeStatus(output: OutputStream, code: Int, reason: String, body: String) {
        val bytes = body.toByteArray()
        val head = "HTTP/1.1 $code $reason\r\nContent-Type: text/plain\r\n" +
            "Content-Length: ${bytes.size}\r\nConnection: close\r\n\r\n"
        output.write(head.toByteArray())
        output.write(bytes)
        output.flush()
    }

    private fun readLine(input: InputStream): String? {
        val builder = StringBuilder()
        while (true) {
            val value = input.read()
            if (value < 0) return if (builder.isEmpty()) null else builder.toString()
            if (value == 10) {
                if (builder.endsWith("\r")) builder.setLength(builder.length - 1)
                return builder.toString()
            }
            builder.append(value.toChar())
            if (builder.length > 8_192) return builder.toString()
        }
    }

    private fun pickVideoFile(storage: FileStorage): Int {
        var best = -1
        var bestSize = 0L
        var sample = -1
        var sampleSize = 0L
        for (index in 0 until storage.numFiles()) {
            val size = runCatching { storage.fileSize(index) }.getOrDefault(0L)
            if (size <= 0) continue
            val path = runCatching { storage.filePath(index) }.getOrNull() ?: continue
            val name = path.lowercase(Locale.US)
            if (VIDEO_TYPES.none { name.endsWith(it) }) continue
            if (name.contains("sample")) {
                if (size > sampleSize) {
                    sample = index
                    sampleSize = size
                }
                continue
            }
            if (size > bestSize) {
                best = index
                bestSize = size
            }
        }
        if (best >= 0) return best
        if (sample >= 0) return sample
        for (index in 0 until storage.numFiles()) {
            val size = runCatching { storage.fileSize(index) }.getOrDefault(0L)
            if (size > bestSize) {
                best = index
                bestSize = size
            }
        }
        return best
    }

    private fun mimeFor(name: String): String {
        val lower = name.lowercase(Locale.US)
        return when {
            lower.endsWith(".mp4") || lower.endsWith(".m4v") -> "video/mp4"
            lower.endsWith(".mkv") -> "video/x-matroska"
            lower.endsWith(".webm") -> "video/webm"
            lower.endsWith(".avi") -> "video/x-msvideo"
            lower.endsWith(".ts") || lower.endsWith(".m2ts") -> "video/mp2t"
            lower.endsWith(".mov") -> "video/quicktime"
            else -> "application/octet-stream"
        }
    }

    private companion object {
        const val METADATA_SECONDS = 60
        const val PREBUFFER_SECONDS = 90
        const val RANGE_SECONDS = 90
        val TRACKERS = listOf(
            "udp://tracker.opentrackr.org:1337/announce",
            "udp://open.demonii.com:1337/announce",
            "udp://tracker.torrent.eu.org:451/announce",
            "udp://exodus.desync.com:6969/announce",
            "udp://tracker.openbittorrent.com:6969/announce",
        )
        val VIDEO_TYPES = listOf(
            ".mp4", ".m4v", ".mkv", ".webm", ".avi", ".mov", ".ts", ".m2ts", ".flv", ".wmv",
        )
    }
}
