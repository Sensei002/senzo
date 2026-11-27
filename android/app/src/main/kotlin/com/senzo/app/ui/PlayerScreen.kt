package com.senzo.app.ui

import android.app.Activity
import android.content.Context
import android.util.TypedValue
import android.view.WindowManager
import androidx.activity.compose.BackHandler
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Audiotrack
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Forward10
import androidx.compose.material.icons.rounded.Pause
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Replay10
import androidx.compose.material.icons.rounded.SkipNext
import androidx.compose.material.icons.rounded.Speed
import androidx.compose.material.icons.rounded.Subtitles
import androidx.compose.material.icons.rounded.ZoomIn
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.TrackGroup
import androidx.media3.common.TrackSelectionOverride
import androidx.media3.common.Tracks
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.HttpDataSource
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.exoplayer.DefaultLoadControl
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.Renderer
import androidx.media3.exoplayer.audio.AudioRendererEventListener
import androidx.media3.exoplayer.audio.AudioSink
import androidx.media3.exoplayer.mediacodec.MediaCodecRenderer
import androidx.media3.exoplayer.mediacodec.MediaCodecSelector
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.exoplayer.source.UnrecognizedInputFormatException
import androidx.media3.ui.AspectRatioFrameLayout
import androidx.media3.ui.PlayerView
import com.senzo.app.Playback
import com.senzo.app.PlayerEngine
import androidx.media3.decoder.ffmpeg.FfmpegAudioRenderer
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.delay
import okhttp3.OkHttpClient

/**
 * Full-screen playback, on whichever engine the settings pick.
 *
 * `libmpv` is what the desktop client runs: it decodes everything itself, so
 * AC3, DTS and ASS subtitles work on any phone. The Media3 player stays the
 * default because it uses the device's hardware decoders.
 */
@Composable
fun PlayerScreen(
    playback: Playback,
    subtitleSize: Int,
    autoNext: Boolean,
    engine: PlayerEngine,
    onProgress: (Long, Long) -> Unit,
    onNext: () -> Unit,
    onRetry: () -> Unit,
    onClose: (Long, Long) -> Unit,
) {
    var libmpv by remember(playback.url, playback.torrent) { mutableStateOf(engine == PlayerEngine.Libmpv) }
    if (libmpv) {
        MpvPlayerScreen(
            playback = playback,
            autoNext = autoNext,
            onProgress = onProgress,
            onNext = onNext,
            onRetry = onRetry,
            onClose = onClose,
            subtitleSize = subtitleSize,
            onUnavailable = { libmpv = false },
        )
    } else {
        ExoPlayerScreen(
            playback = playback,
            subtitleSize = subtitleSize,
            autoNext = autoNext,
            onProgress = onProgress,
            onNext = onNext,
            onRetry = onRetry,
            onClose = onClose,
        )
    }
}

/**
 * The Media3 player, with the app's own controls.
 *
 * A torrent opens this screen before the engine has an address, so the logo
 * breathes over the engine's own status until the first pieces arrive. The
 * controls mirror the desktop client: transport, seek, speed, audio and
 * subtitle tracks, zoom, and the next episode.
 */
@OptIn(UnstableApi::class)
@Composable
private fun ExoPlayerScreen(
    playback: Playback,
    subtitleSize: Int,
    autoNext: Boolean,
    onProgress: (Long, Long) -> Unit,
    onNext: () -> Unit,
    onRetry: () -> Unit,
    onClose: (Long, Long) -> Unit,
) {
    val context = LocalContext.current
    var position by remember { mutableStateOf(playback.startPositionMs) }
    var duration by remember { mutableStateOf(0L) }
    var buffered by remember { mutableStateOf(0L) }
    var playing by remember { mutableStateOf(false) }
    var buffering by remember { mutableStateOf(true) }
    var ended by remember { mutableStateOf(false) }
    var error by remember(playback.url) { mutableStateOf<String?>(null) }
    var scrubbing by remember { mutableStateOf(false) }
    var scrubValue by remember { mutableStateOf(0f) }
    var speed by remember { mutableStateOf(1f) }
    var zoom by remember { mutableStateOf(false) }
    var controls by remember { mutableStateOf(true) }
    var locked by remember { mutableStateOf(false) }
    var sheet by remember { mutableStateOf<PlayerSheet?>(null) }
    var tracks by remember { mutableStateOf<Tracks?>(null) }
    var attempts by remember(playback.url) { mutableStateOf(0) }
    var resumed by remember(playback.url) { mutableStateOf(false) }

    val url = playback.url
    val exoPlayer = remember(url, attempts) {
        if (url == null) {
            null
        } else {
            val headers = playback.headers.filterValues { it.isNotBlank() }
            val dataSource = OkHttpDataSource.Factory(playbackClient)
                .setDefaultRequestProperties(headers)
                .setUserAgent("SENZO Android")
            ExoPlayer.Builder(context)
                .setMediaSourceFactory(DefaultMediaSourceFactory(dataSource))
                .setRenderersFactory(SenzoRenderersFactory(context))
                .setLoadControl(senzoLoadControl())
                .setHandleAudioBecomingNoisy(true)
                .build()
                .apply {
                    setAudioAttributes(
                        androidx.media3.common.AudioAttributes.Builder()
                            .setUsage(C.USAGE_MEDIA)
                            .setContentType(C.AUDIO_CONTENT_TYPE_MOVIE)
                            .build(),
                        true,
                    )
                    setMediaItem(MediaItem.fromUri(url))
                    playWhenReady = true
                    prepare()
                }
        }
    }

    // Back leaves the player the same way the close button does, saving where
    // the viewer stopped.
    BackHandler { onClose(position, duration) }

    // The screen stays awake and the bars step aside while the player is open.
    DisposableEffect(context) {
        val activity = context as? Activity
        val window = activity?.window
        window?.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val bars = activity?.let {
            androidx.core.view.WindowInsetsControllerCompat(it.window, it.window.decorView)
        }
        bars?.systemBarsBehavior =
            androidx.core.view.WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        bars?.hide(androidx.core.view.WindowInsetsCompat.Type.systemBars())
        onDispose {
            window?.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            bars?.show(androidx.core.view.WindowInsetsCompat.Type.systemBars())
        }
    }

    DisposableEffect(exoPlayer) {
        val player = exoPlayer ?: return@DisposableEffect onDispose { }
        val listener = object : Player.Listener {
            override fun onIsPlayingChanged(isPlaying: Boolean) {
                playing = isPlaying
            }

            override fun onPlaybackStateChanged(state: Int) {
                buffering = state == Player.STATE_BUFFERING
                if (state == Player.STATE_READY) {
                    duration = player.duration.coerceAtLeast(0)
                    if (!resumed && playback.startPositionMs > 0) {
                        resumed = true
                        player.seekTo(playback.startPositionMs)
                    }
                }
                if (state == Player.STATE_ENDED) ended = true
            }

            override fun onTracksChanged(playerTracks: Tracks) {
                tracks = playerTracks
            }

            override fun onPlayerError(playerError: PlaybackException) {
                error = describe(playerError)
            }
        }
        player.addListener(listener)
        onDispose {
            player.clearVideoSurface()
            player.release()
        }
    }

    LaunchedEffect(exoPlayer) {
        val player = exoPlayer ?: return@LaunchedEffect
        var ticks = 0
        while (true) {
            delay(400)
            if (!scrubbing) position = player.currentPosition.coerceAtLeast(0)
            duration = player.duration.coerceAtLeast(0)
            buffered = player.bufferedPosition.coerceAtLeast(0)
            if (ticks % 10 == 0 && duration > 0 && position > 0) {
                onProgress(position, duration)
            }
            ticks++
        }
    }

    LaunchedEffect(controls, playing, scrubbing, sheet) {
        if (controls && playing && !scrubbing && sheet == null) {
            delay(4_500)
            controls = false
        }
    }

    LaunchedEffect(ended) {
        if (ended) {
            if (autoNext && playback.hasNext) onNext() else controls = true
        }
    }

    Box(Modifier.fillMaxSize().background(Color.Black)) {
        if (exoPlayer != null) {
            AndroidView(
                factory = { viewContext ->
                    PlayerView(viewContext).apply {
                        player = exoPlayer
                        useController = false
                        setShowBuffering(PlayerView.SHOW_BUFFERING_NEVER)
                        resizeMode = AspectRatioFrameLayout.RESIZE_MODE_FIT
                        setBackgroundColor(android.graphics.Color.BLACK)
                        val size = when (subtitleSize) {
                            0 -> 14f
                            2 -> 24f
                            else -> 18f
                        }
                        subtitleView?.setFixedTextSize(TypedValue.COMPLEX_UNIT_SP, size)
                    }
                },
                update = { view ->
                    // A retry builds a new player, so the surface follows it.
                    view.player = exoPlayer
                    view.resizeMode = if (zoom) {
                        AspectRatioFrameLayout.RESIZE_MODE_ZOOM
                    } else {
                        AspectRatioFrameLayout.RESIZE_MODE_FIT
                    }
                },
                modifier = Modifier.fillMaxSize(),
            )
        }

        // Taps toggle the overlay; a double tap on either half seeks.
        Box(
            Modifier
                .fillMaxSize()
                .pointerInput(controls, exoPlayer) {
                    detectTapGestures(
                        onTap = { controls = !controls },
                        onDoubleTap = { offset ->
                            val player = exoPlayer ?: return@detectTapGestures
                            val delta = if (offset.x < size.width / 2) -10_000 else 10_000
                            player.seekTo((player.currentPosition + delta).coerceAtLeast(0))
                            controls = true
                        },
                    )
                },
        )

        val waiting = url == null || playback.failure != null
        if (waiting) {
            BreathingLogo(
                status = playback.failure ?: playback.status ?: "Preparing the stream…",
                failed = playback.failure != null,
            )
        }

        if (url != null && buffering && error == null && playback.failure == null && !ended) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    CircularProgressIndicator(
                        color = Color.White,
                        strokeWidth = 2.dp,
                        modifier = Modifier.size(38.dp),
                    )
                    // The swarm numbers keep showing during a restart, which is
                    // what tells a slow torrent apart from a stuck player.
                    playback.stats?.let { stats ->
                        Spacer(Modifier.height(14.dp))
                        Text(
                            text = stats,
                            color = Color.White.copy(alpha = 0.8f),
                            style = MaterialTheme.typography.labelMedium,
                            textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                        )
                    }
                }
            }
        }

        if (error != null) {
            FailureOverlay(
                message = error.orEmpty(),
                onRetry = {
                    error = null
                    attempts++
                    onRetry()
                },
                onClose = { onClose(position, duration) },
            )
        }

        if (url == null && playback.failure == null) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .statusBarsPadding()
                    .padding(horizontal = 6.dp, vertical = 6.dp),
            ) {
                IconButton(onClick = { onClose(position, duration) }) {
                    Icon(Icons.Rounded.Close, contentDescription = "Close player", tint = Color.White)
                }
            }
        }

        if (exoPlayer != null && controls) {
            PlayerChrome(
                title = playback.title,
                subtitle = listOfNotNull(
                    playback.year,
                    playback.subtitle.takeIf { it.isNotBlank() },
                ).joinToString("  ·  "),
                playing = playing,
                ended = ended,
                position = position,
                duration = duration,
                buffered = buffered,
                zoom = zoom,
                locked = locked,
                hasNext = playback.hasNext,
                onPlayPause = {
                    if (ended) {
                        exoPlayer.seekTo(0)
                        exoPlayer.play()
                        ended = false
                    } else if (playing) {
                        exoPlayer.pause()
                    } else {
                        exoPlayer.play()
                    }
                    controls = true
                },
                onSeekBy = { delta ->
                    exoPlayer.seekTo((exoPlayer.currentPosition + delta).coerceAtLeast(0))
                    controls = true
                },
                onSeek = { value ->
                    exoPlayer.seekTo(value)
                    position = value
                    controls = true
                },
                onZoom = { zoom = !zoom },
                onSubtitles = { sheet = PlayerSheet.Subtitles },
                onAudio = { sheet = PlayerSheet.Audio },
                onSpeed = { sheet = PlayerSheet.Speed },
                onInfo = { sheet = PlayerSheet.Info },
                onNext = onNext,
                onRetry = {
                    error = null
                    attempts++
                    onRetry()
                },
                onLock = { locked = !locked; controls = true },
                onBack = { onClose(position, duration) },
            )
        }
    }

    val openSheet = sheet
    if (openSheet != null && exoPlayer != null) {
        PlayerSheetContent(
            sheet = openSheet,
            player = exoPlayer,
            tracks = tracks,
            playback = playback,
            position = position,
            duration = duration,
            buffered = buffered,
            speed = speed,
            onSpeed = { value ->
                speed = value
                exoPlayer.setPlaybackSpeed(value)
            },
            onDismiss = { sheet = null },
        )
    }
}

/**
 * Media needs its own client: the shared one caps a call at 45 seconds, which
 * would cut a film mid-stream. Reads wait while the torrent fills a piece.
 */
private val playbackClient: OkHttpClient by lazy {
    OkHttpClient.Builder()
        .connectTimeout(20, TimeUnit.SECONDS)
        .readTimeout(120, TimeUnit.SECONDS)
        .retryOnConnectionFailure(true)
        .build()
}

/** Which settings sheet is open. */
private enum class PlayerSheet { Speed, Audio, Subtitles, Info }

/**
 * The buffer the desktop app gives a stream, sized for a phone: enough ahead of
 * a torrent to survive a slow peer, small enough for a few hundred MB of RAM.
 */
@OptIn(UnstableApi::class)
private fun senzoLoadControl(): DefaultLoadControl = DefaultLoadControl.Builder()
    .setBufferDurationsMs(
        15_000,
        60_000,
        2_500,
        4_000,
    )
    .setTargetBufferBytes(64 * 1024 * 1024)
    .setPrioritizeTimeOverSizeThresholds(false)
    .setBackBuffer(30_000, true)
    .build()

/**
 * Adds the FFmpeg audio decoder in front of the platform ones. The platform
 * decoders stay for AAC and MP3, which they handle in hardware; AC3, EAC3,
 * DTS and TrueHD fall through to FFmpeg, which is why a stream that used to
 * play silently now has sound.
 */
@OptIn(UnstableApi::class)
private class SenzoRenderersFactory(context: Context) : DefaultRenderersFactory(context) {
    init {
        // Media3 starts with extensions switched off, which silently dropped every
        // FFmpeg audio renderer: this is what left AC3 and DTS streams silent.
        setExtensionRendererMode(EXTENSION_RENDERER_MODE_ON)
        setEnableDecoderFallback(true)
    }

    override fun buildAudioRenderers(
        context: Context,
        extensionRendererMode: Int,
        mediaCodecSelector: MediaCodecSelector,
        enableDecoderFallback: Boolean,
        audioSink: AudioSink,
        eventHandler: android.os.Handler,
        eventListener: AudioRendererEventListener,
        out: ArrayList<Renderer>,
    ) {
        super.buildAudioRenderers(
            context,
            extensionRendererMode,
            mediaCodecSelector,
            enableDecoderFallback,
            audioSink,
            eventHandler,
            eventListener,
            out,
        )
        // The bundled decoder lives in its own package, so Media3's own extension
        // lookup never finds it; it is added here, ahead of the platform decoders.
        runCatching { FfmpegAudioRenderer(eventHandler, eventListener, audioSink) }
            .onSuccess { out.add(0, it) }
    }
}

@Composable
internal fun BreathingLogo(status: String, failed: Boolean) {
    val transition = rememberInfiniteTransition(label = "logo")
    val scale by transition.animateFloat(
        initialValue = 0.94f,
        targetValue = 1.06f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = 1_400),
            repeatMode = RepeatMode.Reverse,
        ),
        label = "breath",
    )
    val alpha by transition.animateFloat(
        initialValue = 0.55f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = 1_400),
            repeatMode = RepeatMode.Reverse,
        ),
        label = "glow",
    )
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Image(
                painter = painterResource(com.senzo.app.R.drawable.senzo_mark),
                contentDescription = "SENZO",
                modifier = Modifier
                    .size(128.dp)
                    .scale(if (failed) 1f else scale)
                    .clip(RoundedCornerShape(24.dp)),
                alpha = if (failed) 0.35f else alpha,
            )
            Spacer(Modifier.height(20.dp))
            Text(
                text = status,
                style = MaterialTheme.typography.bodyMedium,
                color = if (failed) Color(0xFFFF6B6B) else TextMuted.copy(alpha = alpha),
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.padding(horizontal = 32.dp),
            )
            if (!failed) {
                Spacer(Modifier.height(18.dp))
                CircularProgressIndicator(
                    color = Accent,
                    strokeWidth = 2.dp,
                    modifier = Modifier.size(22.dp),
                )
            }
        }
    }
}

@Composable
internal fun FailureOverlay(message: String, onRetry: () -> Unit, onClose: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize().padding(28.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Text(
            text = "This stream could not be played.",
            style = MaterialTheme.typography.titleMedium,
            color = Color.White,
        )
        Spacer(Modifier.height(6.dp))
        Text(
            text = message,
            style = MaterialTheme.typography.bodySmall,
            color = TextMuted,
        )
        Spacer(Modifier.height(18.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            TextButton(onClick = onRetry) { Text("Try again", color = Accent) }
            TextButton(onClick = onClose) { Text("Choose another source", color = TextMuted) }
        }
    }
}

@Composable
internal fun ControlPill(
    text: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    onClick: () -> Unit,
) {
    Row(
        modifier = Modifier
            .clip(RoundedCornerShape(50))
            .background(Color.White.copy(alpha = 0.14f))
            .clickable(onClick = onClick)
            .padding(horizontal = 10.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, contentDescription = null, tint = Color.White, modifier = Modifier.size(14.dp))
        Spacer(Modifier.width(5.dp))
        Text(
            text = text,
            style = MaterialTheme.typography.labelSmall,
            color = Color.White,
            maxLines = 1,
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PlayerSheetContent(
    sheet: PlayerSheet,
    player: ExoPlayer,
    tracks: Tracks?,
    playback: Playback,
    position: Long,
    duration: Long,
    buffered: Long,
    speed: Float,
    onSpeed: (Float) -> Unit,
    onDismiss: () -> Unit,
) {
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = state,
        containerColor = Surface,
    ) {
        Column(Modifier.fillMaxWidth().padding(bottom = 24.dp)) {
            Text(
                text = when (sheet) {
                    PlayerSheet.Speed -> "Playback speed"
                    PlayerSheet.Audio -> "Audio track"
                    PlayerSheet.Subtitles -> "Subtitles"
                    PlayerSheet.Info -> "Stream details"
                },
                style = MaterialTheme.typography.titleMedium,
                color = TextPrimary,
                modifier = Modifier.padding(horizontal = 20.dp, vertical = 12.dp),
            )
            when (sheet) {
                PlayerSheet.Speed -> Row(
                    modifier = Modifier.padding(horizontal = 16.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    listOf(0.5f, 0.75f, 1f, 1.25f, 1.5f, 2f).forEach { value ->
                        Text(
                            text = formatSpeed(value),
                            style = MaterialTheme.typography.labelMedium,
                            color = if (value == speed) Color.White else TextMuted,
                            modifier = Modifier
                                .clip(RoundedCornerShape(50))
                                .background(if (value == speed) Accent else SurfaceHigh)
                                .clickable { onSpeed(value); onDismiss() }
                                .padding(horizontal = 14.dp, vertical = 8.dp),
                        )
                    }
                }
                PlayerSheet.Info -> {
                    InfoRow("Playing", playback.title)
                    if (!playback.year.isNullOrBlank()) InfoRow("Released", playback.year.orEmpty())
                    if (playback.subtitle.isNotBlank()) InfoRow("This stream", playback.subtitle)
                    InfoRow("Engine", "Media3 ExoPlayer")
                    InfoRow("Position", formatTime(position) + " of " + formatTime(duration))
                    InfoRow("Buffered", formatTime(buffered) + " ahead")
                    InfoRow("Speed", formatSpeed(speed))
                    playback.stats?.let { InfoRow("Swarm", it) }
                }
                PlayerSheet.Audio, PlayerSheet.Subtitles -> {
                    val type = if (sheet == PlayerSheet.Audio) C.TRACK_TYPE_AUDIO else C.TRACK_TYPE_TEXT
                    val groups = trackGroups(tracks, type)
                    if (groups.isEmpty()) {
                        Text(
                            text = if (type == C.TRACK_TYPE_AUDIO) {
                                "This file has a single audio track."
                            } else {
                                "This file has no subtitle track."
                            },
                            style = MaterialTheme.typography.bodySmall,
                            color = TextMuted,
                            modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp),
                        )
                    } else {
                        if (type == C.TRACK_TYPE_TEXT) {
                            TrackRow(
                                label = "Off",
                                selected = trackLabel(tracks, C.TRACK_TYPE_TEXT) == null,
                                onClick = {
                                    player.trackSelectionParameters = player.trackSelectionParameters
                                        .buildUpon()
                                        .setTrackTypeDisabled(C.TRACK_TYPE_TEXT, true)
                                        .build()
                                    onDismiss()
                                },
                            )
                        }
                        groups.forEach { group ->
                            TrackRow(
                                label = trackName(group, 0)
                                    ?: (if (type == C.TRACK_TYPE_AUDIO) "Audio" else "Subtitle"),
                                selected = player.trackSelectionParameters.overrides
                                    .containsKey(group),
                                onClick = {
                                    player.trackSelectionParameters = player.trackSelectionParameters
                                        .buildUpon()
                                        .setTrackTypeDisabled(type, false)
                                        .setOverrideForType(TrackSelectionOverride(group, 0))
                                        .build()
                                    onDismiss()
                                },
                            )
                        }
                    }
                }
            }
        }
    }
}

/** One label and value line in the stream details sheet. */
@Composable
private fun InfoRow(label: String, value: String) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.bodySmall,
            color = TextMuted,
            modifier = Modifier.width(92.dp),
        )
        Text(
            text = value,
            style = MaterialTheme.typography.bodyMedium,
            color = TextPrimary,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
internal fun TrackRow(label: String, selected: Boolean, onClick: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 20.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.bodyMedium,
            color = if (selected) Accent else TextPrimary,
            modifier = Modifier.weight(1f),
        )
        if (selected) {
            Text("Selected", style = MaterialTheme.typography.labelSmall, color = Accent)
        }
    }
}

private fun trackGroups(tracks: Tracks?, type: Int): List<TrackGroup> {
    val playerTracks = tracks ?: return emptyList()
    val groups = mutableListOf<TrackGroup>()
    for (group in playerTracks.groups) {
        if (group.type == type && group.isSupported) {
            groups.add(group.mediaTrackGroup)
        }
    }
    return groups
}

private fun trackLabel(tracks: Tracks?, type: Int): String? {
    val playerTracks = tracks ?: return null
    for (group in playerTracks.groups) {
        if (group.type != type || !group.isSupported) continue
        for (index in 0 until group.length) {
            if (group.isTrackSelected(index)) {
                return trackName(group.mediaTrackGroup, index)
            }
        }
    }
    return null
}

private fun trackName(group: TrackGroup, index: Int): String? {
    val format = group.getFormat(index)
    val language = format.language
    val label = format.label
    if (!label.isNullOrBlank()) return label
    if (!language.isNullOrBlank()) return language.uppercase()
    return format.sampleMimeType
}

/**
 * Turns a Media3 failure into what the viewer needs: the HTTP status when the
 * host refused the request, the decoder that failed, or the format problem.
 */
@OptIn(UnstableApi::class)
private fun describe(error: PlaybackException): String {
    val cause = error.cause
    if (cause is HttpDataSource.InvalidResponseCodeException) {
        val reason = when (cause.responseCode) {
            401, 410 -> "The stream link expired. Pick the source again."
            403 -> "The stream host blocked this device."
            404 -> "The stream host no longer has this file."
            429 -> "The stream host is rate limiting requests."
            in 500..599 -> "The stream host is having trouble right now."
            else -> "The stream host refused the request."
        }
        return "HTTP ${cause.responseCode} · $reason"
    }
    var current: Throwable? = cause
    while (current != null) {
        if (current is UnrecognizedInputFormatException) {
            return "This source did not return a video file."
        }
        if (current is MediaCodecRenderer.DecoderInitializationException) {
            val detail = current.message?.trim()?.takeIf { it.isNotEmpty() }
                ?: current.diagnosticInfo?.trim()?.takeIf { it.isNotEmpty() }
            return if (detail != null) "The decoder failed: $detail" else "This device cannot decode the file."
        }
        current = current.cause
    }
    return when (error.errorCode) {
        PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_FAILED -> "The stream host could not be reached."
        PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_TIMEOUT -> "The stream host timed out."
        PlaybackException.ERROR_CODE_IO_BAD_HTTP_STATUS -> "The stream host refused the request."
        PlaybackException.ERROR_CODE_DECODER_INIT_FAILED -> "This device cannot decode the file."
        PlaybackException.ERROR_CODE_DECODING_FAILED -> "The file could not be decoded."
        PlaybackException.ERROR_CODE_PARSING_CONTAINER_UNSUPPORTED -> "The file format is not supported."
        else -> error.errorCodeName
    }
}

internal fun formatTime(milliseconds: Long): String {
    val total = (milliseconds / 1000).coerceAtLeast(0)
    val hours = total / 3600
    val minutes = (total % 3600) / 60
    val seconds = total % 60
    return if (hours > 0) {
        "%d:%02d:%02d".format(hours, minutes, seconds)
    } else {
        "%d:%02d".format(minutes, seconds)
    }
}

internal fun formatSpeed(speed: Float): String =
    if (speed == speed.toInt().toFloat()) "${speed.toInt()}x" else "${speed}x"

internal fun nextSpeed(speed: Float): Float = when (speed) {
    0.75f -> 1f
    1f -> 1.25f
    1.25f -> 1.5f
    1.5f -> 2f
    else -> 0.75f
}
