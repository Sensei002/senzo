package com.senzo.app.player

import android.content.Context
import android.view.SurfaceHolder
import `is`.xyz.mpv.BaseMPVView
import `is`.xyz.mpv.MPV
import java.io.File

/** One audio or subtitle track as mpv lists it. */
data class MpvTrack(
    val id: Long,
    val type: String,
    val title: String?,
    val language: String?,
    val codec: String?,
    val selected: Boolean,
) {
    /** What the pills show: the title when there is one, else the language. */
    val label: String
        get() = listOfNotNull(
            title?.trim()?.takeIf { it.isNotEmpty() },
            language?.trim()?.takeIf { it.isNotEmpty() }?.uppercase(),
        ).firstOrNull() ?: codec?.uppercase() ?: "Track $id"
}

/** Everything the controls need, read back from mpv in one go. */
data class MpvSnapshot(
    val positionMs: Long = 0,
    val durationMs: Long = 0,
    val cachedMs: Long = 0,
    val paused: Boolean = false,
    val buffering: Boolean = false,
    val ended: Boolean = false,
    val speed: Float = 1f,
    val audio: List<MpvTrack> = emptyList(),
    val subtitles: List<MpvTrack> = emptyList(),
    val error: String? = null,
) {
    val hasAudio: Boolean get() = audio.isNotEmpty()
}

/**
 * The libmpv engine, the one the desktop client runs.
 *
 * mpv does the decoding, demuxing and subtitle rendering itself, which is why
 * it plays AC3, DTS, TrueHD and ASS subtitles on phones that have no decoder
 * for them. This view only owns the surface and translates commands; the
 * Compose overlay draws the controls from [poll].
 */
class MpvPlayerView(context: Context) : BaseMPVView(context, null) {

    @Volatile
    private var lastError: String? = null

    @Volatile
    private var closed = false

    private var pendingUrl: String? = null
    private var pendingStartMs: Long = 0
    private var caFile: File? = null

    /** mpv numbers its levels the other way round: 20 is an error, 30 a warning. */
    private val logger = object : MPV.LogObserver {
        override fun logMessage(prefix: String, level: Int, text: String) {
            if (level <= 30 && text.isNotBlank()) lastError = text.trim()
        }
    }

    /** Creates the player. Call once, before the view is laid out. */
    fun start(configDir: File, cacheDir: File) {
        configDir.mkdirs()
        cacheDir.mkdirs()
        caFile = copyCaCertificates()
        runCatching { mpv.addLogObserver(logger) }
        initialize(configDir.path, cacheDir.path)
    }

    /** Opens a stream, keeping the address until the surface exists. */
    fun play(url: String, headers: Map<String, String>, startMs: Long) {
        pendingUrl = url
        pendingStartMs = startMs
        lastError = null
        applyHeaders(headers)
        if (holder.surface?.isValid == true) sendLoad()
    }

    override fun surfaceCreated(holder: SurfaceHolder) {
        super.surfaceCreated(holder)
        sendLoad()
    }

    private fun sendLoad() {
        val url = pendingUrl ?: return
        pendingUrl = null
        mpv.command("loadfile", url, "replace")
        val resume = pendingStartMs
        pendingStartMs = 0
        if (resume > 5_000) {
            // Seeking once the stream reports a duration is more reliable than
            // mpv start options on remuxed torrents.
            mpv.command("seek", (resume / 1000.0).toString(), "absolute")
        }
    }

    /** Renders the browser headers a stream host expects. */
    private fun applyHeaders(headers: Map<String, String>) {
        val fields = headers
            .filterValues { it.isNotBlank() }
            .map { (name, value) -> "$name: $value" }
        runCatching { mpv.setPropertyString("http-header-fields", fields.joinToString(",")) }
    }

    override fun initOptions() {
        mpv.setOptionString("vo", "gpu")
        mpv.setOptionString("gpu-context", "android")
        mpv.setOptionString("hwdec", "auto-safe")
        mpv.setOptionString("ao", "audiotrack")
        mpv.setOptionString("msg-level", "all=warn")
        mpv.setOptionString("ytdl", "no")
        mpv.setOptionString("tls-verify", "yes")
        caFile?.let { mpv.setOptionString("tls-ca-file", it.path) }
        mpv.setOptionString("cache", "yes")
        mpv.setOptionString("demuxer-max-bytes", "64MiB")
        mpv.setOptionString("demuxer-max-back-bytes", "32MiB")
        mpv.setOptionString("user-agent", "SENZO Android")
        mpv.setOptionString("audio-fallback-to-null", "yes")
        mpv.setOptionString("force-seekable", "yes")
        mpv.setOptionString("sub-auto", "no")
    }

    override fun postInitOptions() = Unit

    override fun observeProperties() = Unit

    /** The mpv log carries the reason a stream did not open. */
    fun error(): String? = if (closed) null else lastError

    fun togglePause() {
        if (closed) return
        val paused = mpv.getPropertyBoolean("pause") ?: false
        mpv.setPropertyBoolean("pause", !paused)
    }

    fun seekTo(milliseconds: Long) {
        if (closed) return
        mpv.command("seek", (milliseconds / 1000.0).toString(), "absolute")
    }

    fun seekBy(deltaMilliseconds: Long) {
        if (closed) return
        mpv.command("seek", (deltaMilliseconds / 1000.0).toString(), "relative")
    }

    fun setSpeed(speed: Float) {
        if (closed) return
        mpv.setPropertyDouble("speed", speed.toDouble())
    }

    fun selectAudio(id: Long?) {
        if (closed) return
        mpv.setPropertyString("aid", id?.toString() ?: "no")
    }

    fun selectSubtitle(id: Long?) {
        if (closed) return
        mpv.setPropertyString("sid", id?.toString() ?: "no")
    }

    fun poll(): MpvSnapshot {
        if (closed) return MpvSnapshot()
        val duration = mpv.getPropertyDouble("duration") ?: 0.0
        val position = mpv.getPropertyDouble("time-pos") ?: 0.0
        val paused = mpv.getPropertyBoolean("pause") ?: false
        val cachePause = mpv.getPropertyBoolean("paused-for-cache") ?: false
        val idle = mpv.getPropertyBoolean("core-idle") ?: false
        val ended = mpv.getPropertyBoolean("eof-reached") ?: false
        val speed = (mpv.getPropertyDouble("speed") ?: 1.0).toFloat()
        val cached = mpv.getPropertyDouble("demuxer-cache-time") ?: 0.0
        val tracks = readTracks()
        return MpvSnapshot(
            positionMs = (position * 1000).toLong().coerceAtLeast(0),
            durationMs = (duration * 1000).toLong().coerceAtLeast(0),
            cachedMs = (cached * 1000).toLong().coerceAtLeast(0),
            paused = paused,
            buffering = cachePause || (idle && !paused),
            ended = ended,
            speed = speed,
            audio = tracks.first,
            subtitles = tracks.second,
            error = lastError,
        )
    }

    private fun readTracks(): Pair<List<MpvTrack>, List<MpvTrack>> {
        val entries = mpv.getPropertyNode("track-list")?.asArray() ?: return emptyList<MpvTrack>() to emptyList()
        val audio = ArrayList<MpvTrack>(4)
        val subtitles = ArrayList<MpvTrack>(4)
        for (entry in entries) {
            val type = entry["type"]?.asString() ?: continue
            if (type != "audio" && type != "sub") continue
            val track = MpvTrack(
                id = entry["id"]?.asInt() ?: continue,
                type = type,
                title = entry["title"]?.asString(),
                language = entry["lang"]?.asString(),
                codec = entry["codec"]?.asString(),
                selected = entry["selected"]?.asBoolean() ?: false,
            )
            if (type == "audio") audio += track else subtitles += track
        }
        return audio to subtitles
    }

    /** Closes the player once, off the surface callback. */
    fun close() {
        if (closed) return
        closed = true
        runCatching { mpv.removeLogObserver(logger) }
        runCatching { destroy() }
    }

    /** mpv needs real CA roots on Android; the AAR ships them as an asset. */
    private fun copyCaCertificates(): File? = runCatching {
        val target = File(context.filesDir, "cacert.pem")
        if (!target.exists() || target.length() < 1024) {
            context.assets.open("cacert.pem").use { input ->
                target.outputStream().use { output -> input.copyTo(output) }
            }
        }
        target.takeIf { it.exists() }
    }.getOrNull()
}