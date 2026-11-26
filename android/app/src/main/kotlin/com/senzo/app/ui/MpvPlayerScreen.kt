package com.senzo.app.ui

import android.app.Activity
import android.view.WindowManager
import androidx.activity.compose.BackHandler
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
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import com.senzo.app.Playback
import com.senzo.app.player.MpvPlayerView
import com.senzo.app.player.MpvSnapshot
import com.senzo.app.player.MpvTrack
import java.io.File
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive

/**
 * The libmpv player, the engine the desktop client runs.
 *
 * It opens the same streams as the Media3 player and renders its own
 * subtitles, so ASS styling and formats Android has no decoder for still play.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MpvPlayerScreen(
    playback: Playback,
    autoNext: Boolean,
    onProgress: (Long, Long) -> Unit,
    onNext: () -> Unit,
    onRetry: () -> Unit,
    onClose: (Long, Long) -> Unit,
    onUnavailable: () -> Unit = {},
) {
    val context = LocalContext.current
    var snapshot by remember { mutableStateOf(MpvSnapshot()) }
    var engine by remember { mutableStateOf<MpvPlayerView?>(null) }
    var attempts by remember { mutableStateOf(0) }
    var controls by remember { mutableStateOf(true) }
    var scrubbing by remember { mutableStateOf(false) }
    var scrubValue by remember { mutableStateOf(0f) }
    var sheet by remember { mutableStateOf(false) }
    var failure by remember { mutableStateOf<String?>(null) }
    val unavailable = remember { mutableStateOf(false) }

    val url = playback.url
    val position = snapshot.positionMs
    val duration = snapshot.durationMs

    BackHandler { onClose(position, duration) }

    DisposableEffect(context) {
        val activity = context as? Activity
        activity?.window?.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        hideSystemBars(activity)
        onDispose {
            activity?.window?.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
    }

    // If the native library cannot load, the Media3 player takes over.
    LaunchedEffect(unavailable.value) {
        if (unavailable.value) onUnavailable()
    }

    LaunchedEffect(engine, attempts) {
        while (isActive) {
            val player = engine
            if (player != null) snapshot = player.poll()
            delay(250)
        }
    }

    // A stream that never reports a duration is not going to start.
    LaunchedEffect(attempts) {
        delay(20_000)
        if (snapshot.durationMs == 0L) {
            failure = engine?.error() ?: "The player could not open this stream."
        }
    }

    LaunchedEffect(snapshot.ended) {
        if (!snapshot.ended) return@LaunchedEffect
        if (autoNext && playback.hasNext) onNext() else controls = true
    }

    LaunchedEffect(controls) {
        if (!controls) return@LaunchedEffect
        delay(4_500)
        if (!scrubbing && !snapshot.paused) controls = false
    }

    Box(Modifier.fillMaxSize().background(Color.Black)) {
        key(attempts) {
            AndroidView(
                factory = { ctx ->
                    runCatching {
                        MpvPlayerView(ctx).apply {
                            start(File(ctx.filesDir, "mpv-config"), File(ctx.cacheDir, "mpv"))
                            engine = this
                            if (url != null) play(url, playback.headers, playback.startPositionMs)
                        }
                    }.getOrElse {
                        unavailable.value = true
                        android.view.View(ctx)
                    }
                },
                modifier = Modifier.fillMaxSize(),
                onRelease = { view -> view.close() },
            )
        }

        // Tap toggles the controls, a double tap on either half seeks.
        Box(
            Modifier.fillMaxSize().pointerInput(Unit) {
                detectTapGestures(
                    onTap = {
                        if (scrubbing) return@detectTapGestures
                        controls = !controls
                    },
                    onDoubleTap = { offset ->
                        val delta = if (offset.x < size.width / 2) -10_000L else 10_000L
                        engine?.seekBy(delta)
                        controls = true
                    },
                )
            },
        )

        val waiting = url == null || failure != null
        if (waiting) {
            BreathingLogo(
                status = failure ?: playback.status ?: "Preparing the stream…",
                failed = failure != null,
            )
        }

        if (url != null && failure == null) {
            if (snapshot.buffering && !snapshot.ended) {
                Column(
                    modifier = Modifier.align(Alignment.Center),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    CircularProgressIndicator(
                        color = Color.White,
                        strokeWidth = 2.dp,
                        modifier = Modifier.size(38.dp),
                    )
                    playback.stats?.let { stats ->
                        Spacer(Modifier.height(14.dp))
                        Text(
                            text = stats,
                            color = Color.White.copy(alpha = 0.8f),
                            style = MaterialTheme.typography.labelMedium,
                            textAlign = TextAlign.Center,
                        )
                    }
                }
            }
        }

        if (failure != null) {
            FailureOverlay(
                message = failure.orEmpty(),
                onRetry = {
                    failure = null
                    attempts++
                    onRetry()
                },
                onClose = { onClose(position, duration) },
            )
        }

        if (controls && failure == null && !waiting) {
            Column(Modifier.fillMaxSize()) {
                Box(
                    Modifier
                        .fillMaxWidth()
                        .background(
                            Brush.verticalGradient(
                                0f to Color.Black.copy(alpha = 0.78f),
                                1f to Color.Transparent,
                            ),
                        )
                        .statusBarsPadding()
                        .padding(horizontal = 6.dp, vertical = 6.dp),
                ) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        IconButton(onClick = { onClose(position, duration) }) {
                            Icon(Icons.Rounded.Close, contentDescription = "Close player", tint = Color.White)
                        }
                        Column(Modifier.weight(1f)) {
                            Text(
                                text = playback.title,
                                style = MaterialTheme.typography.titleSmall,
                                color = Color.White,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            if (playback.subtitle.isNotBlank()) {
                                Text(
                                    text = playback.subtitle,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = TextMuted,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            }
                        }
                    }
                }

                Row(
                    modifier = Modifier.weight(1f).fillMaxWidth(),
                    horizontalArrangement = Arrangement.Center,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    IconButton(onClick = { engine?.seekBy(-10_000) }, modifier = Modifier.size(56.dp)) {
                        Icon(Icons.Rounded.Replay10, contentDescription = "Back 10 seconds", tint = Color.White, modifier = Modifier.size(28.dp))
                    }
                    Spacer(Modifier.width(24.dp))
                    Box(
                        modifier = Modifier
                            .size(72.dp)
                            .clip(CircleShape)
                            .background(Color.Black.copy(alpha = 0.45f))
                            .clickable { engine?.togglePause() },
                        contentAlignment = Alignment.Center,
                    ) {
                        if (snapshot.buffering && !snapshot.ended) {
                            CircularProgressIndicator(
                                color = Color.White,
                                strokeWidth = 2.5.dp,
                                modifier = Modifier.size(40.dp),
                            )
                        } else {
                            Icon(
                                imageVector = if (snapshot.paused) Icons.Rounded.PlayArrow else Icons.Rounded.Pause,
                                contentDescription = if (snapshot.paused) "Play" else "Pause",
                                tint = Color.White,
                                modifier = Modifier.size(36.dp),
                            )
                        }
                    }
                    Spacer(Modifier.width(24.dp))
                    IconButton(onClick = { engine?.seekBy(10_000) }, modifier = Modifier.size(56.dp)) {
                        Icon(Icons.Rounded.Forward10, contentDescription = "Forward 10 seconds", tint = Color.White, modifier = Modifier.size(28.dp))
                    }
                }

                Column(
                    Modifier
                        .fillMaxWidth()
                        .background(
                            Brush.verticalGradient(
                                0f to Color.Transparent,
                                1f to Color.Black.copy(alpha = 0.85f),
                            ),
                        )
                        .navigationBarsPadding()
                        .padding(horizontal = 16.dp, vertical = 10.dp),
                ) {
                    val shown = if (scrubbing) scrubValue.toLong() else position
                    Slider(
                        value = if (duration > 0) (shown.toFloat() / duration).coerceIn(0f, 1f) else 0f,
                        onValueChange = {
                            scrubbing = true
                            scrubValue = (it * duration).toLong()
                        },
                        onValueChangeFinished = {
                            engine?.seekTo(scrubValue)
                            scrubbing = false
                        },
                        colors = SliderDefaults.colors(
                            thumbColor = Accent,
                            activeTrackColor = Accent,
                            inactiveTrackColor = Color.White.copy(alpha = 0.28f),
                        ),
                    )
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                    ) {
                        Text(
                            text = "${formatTime(shown)} / ${formatTime(duration)}",
                            style = MaterialTheme.typography.labelMedium,
                            color = TextMuted,
                        )
                        Text(
                            text = "libmpv",
                            style = MaterialTheme.typography.labelSmall,
                            color = TextDim,
                        )
                    }
                    Spacer(Modifier.height(8.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        ControlPill(
                            icon = Icons.Rounded.Speed,
                            text = if (snapshot.speed == 1f) "Speed" else formatSpeed(snapshot.speed),
                            onClick = {
                                val next = nextSpeed(snapshot.speed)
                                engine?.setSpeed(next)
                            },
                        )
                        ControlPill(
                            icon = Icons.Rounded.Audiotrack,
                            text = snapshot.audio.firstOrNull { it.selected }?.label ?: "Audio",
                            onClick = { sheet = true },
                        )
                        ControlPill(
                            icon = Icons.Rounded.Subtitles,
                            text = snapshot.subtitles.firstOrNull { it.selected }?.label ?: "Subs",
                            onClick = { sheet = true },
                        )
                        if (playback.hasNext) {
                            ControlPill(
                                icon = Icons.Rounded.SkipNext,
                                text = "Next",
                                onClick = onNext,
                            )
                        }
                    }
                }
            }
        }

        if (sheet) {
            val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
            ModalBottomSheet(
                onDismissRequest = { sheet = false },
                sheetState = state,
                containerColor = Surface,
                contentColor = TextPrimary,
            ) {
                MpvTracksSheet(
                    snapshot = snapshot,
                    onAudio = { track -> engine?.selectAudio(track?.id) },
                    onSubtitle = { track -> engine?.selectSubtitle(track?.id) },
                )
            }
        }
    }
}

@Composable
private fun MpvTracksSheet(
    snapshot: MpvSnapshot,
    onAudio: (MpvTrack?) -> Unit,
    onSubtitle: (MpvTrack?) -> Unit,
) {
    Column(Modifier.padding(horizontal = 20.dp).padding(bottom = 24.dp)) {
        Text("Audio", style = MaterialTheme.typography.titleSmall, color = TextPrimary)
        snapshot.audio.forEach { track ->
            TrackRow(label = track.label, selected = track.selected) { onAudio(track) }
        }
        Spacer(Modifier.height(16.dp))
        Text("Subtitles", style = MaterialTheme.typography.titleSmall, color = TextPrimary)
        TrackRow(label = "Off", selected = snapshot.subtitles.none { it.selected }) { onSubtitle(null) }
        snapshot.subtitles.forEach { track ->
            TrackRow(label = track.label, selected = track.selected) { onSubtitle(track) }
        }
    }
}

private fun hideSystemBars(activity: Activity?) {
    if (activity == null) return
    val bars = androidx.core.view.WindowInsetsControllerCompat(activity.window, activity.window.decorView)
    bars.systemBarsBehavior = androidx.core.view.WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
    bars.hide(androidx.core.view.WindowInsetsCompat.Type.systemBars())
}