package com.senzo.app.ui

import android.util.TypedValue
import androidx.annotation.OptIn
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Forward10
import androidx.compose.material.icons.rounded.Pause
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Replay10
import androidx.compose.material.icons.rounded.SkipNext
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
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
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.ui.AspectRatioFrameLayout
import androidx.media3.ui.PlayerView
import com.senzo.app.Http
import com.senzo.app.Playback
import kotlinx.coroutines.delay

/**
 * Full-screen playback with the app's own controls.
 *
 * The stream request headers come from the addon (`behaviorHints.proxyHeaders`),
 * which is how addons that need a referer keep working. ExoPlayer keeps its
 * built-in controller off so the overlay can match the rest of the app.
 */
@OptIn(UnstableApi::class)
@Composable
fun PlayerScreen(
    playback: Playback,
    subtitleSize: Int,
    autoNext: Boolean,
    onProgress: (Long, Long) -> Unit,
    onNext: () -> Unit,
    onClose: (Long, Long) -> Unit,
) {
    val context = LocalContext.current
    var error by remember(playback.url) { mutableStateOf<String?>(null) }
    var playing by remember(playback.url) { mutableStateOf(true) }
    var position by remember(playback.url) { mutableStateOf(playback.startPositionMs) }
    var duration by remember(playback.url) { mutableStateOf(0L) }
    var buffering by remember(playback.url) { mutableStateOf(true) }
    var scrubbing by remember(playback.url) { mutableStateOf(false) }
    var scrubValue by remember(playback.url) { mutableStateOf(0f) }
    var speed by remember(playback.url) { mutableStateOf(1f) }
    var zoom by remember(playback.url) { mutableStateOf(false) }
    var controls by remember(playback.url) { mutableStateOf(true) }
    var ended by remember(playback.url) { mutableStateOf(false) }

    val exoPlayer = remember(playback.url) {
        val headers = playback.headers.filterValues { it.isNotBlank() }
        val dataSource = OkHttpDataSource.Factory(Http.client)
            .setDefaultRequestProperties(headers)
            .setUserAgent("SENZO Android")
        ExoPlayer.Builder(context)
            .setMediaSourceFactory(DefaultMediaSourceFactory(dataSource))
            .build()
            .apply {
                setMediaItem(MediaItem.fromUri(playback.url))
                playWhenReady = true
                prepare()
                if (playback.startPositionMs > 0) seekTo(playback.startPositionMs)
            }
    }

    DisposableEffect(exoPlayer, playback.url) {
        val listener = object : Player.Listener {
            override fun onIsPlayingChanged(isPlaying: Boolean) {
                playing = isPlaying
            }

            override fun onPlaybackStateChanged(state: Int) {
                buffering = state == Player.STATE_BUFFERING
                if (state == Player.STATE_READY) {
                    duration = exoPlayer.duration.coerceAtLeast(0)
                }
                if (state == Player.STATE_ENDED) {
                    ended = true
                }
            }

            override fun onPlayerError(playerError: PlaybackException) {
                error = playerError.errorCodeName
            }
        }
        exoPlayer.addListener(listener)
        onDispose {
            exoPlayer.removeListener(listener)
            exoPlayer.release()
        }
    }

    LaunchedEffect(exoPlayer, playback.url) {
        var ticks = 0
        while (true) {
            delay(500)
            if (!scrubbing) {
                position = exoPlayer.currentPosition.coerceAtLeast(0)
            }
            val total = exoPlayer.duration
            if (total > 0) duration = total
            if (ticks % 10 == 0 && duration > 0 && position > 0) {
                onProgress(position, duration)
            }
            ticks++
        }
    }

    LaunchedEffect(controls, playing, scrubbing) {
        if (controls && playing && !scrubbing) {
            delay(4_500)
            controls = false
        }
    }

    LaunchedEffect(ended) {
        if (ended) {
            if (autoNext && playback.hasNext) {
                onNext()
            } else {
                controls = true
            }
        }
    }

    Box(Modifier.fillMaxSize().background(Color.Black)) {
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
                view.resizeMode = if (zoom) {
                    AspectRatioFrameLayout.RESIZE_MODE_ZOOM
                } else {
                    AspectRatioFrameLayout.RESIZE_MODE_FIT
                }
            },
            modifier = Modifier.fillMaxSize(),
        )

        // A tap anywhere shows or hides the overlay, the way a player behaves.
        Box(Modifier.fillMaxSize().clickable { controls = !controls })

        if (buffering && error == null) {
            CircularProgressIndicator(
                color = Accent,
                strokeWidth = 2.5.dp,
                modifier = Modifier.align(Alignment.Center).size(42.dp),
            )
        }

        if (error != null) {
            Column(
                modifier = Modifier.align(Alignment.Center).padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text(
                    text = "This stream could not be played.",
                    style = MaterialTheme.typography.titleMedium,
                    color = Color.White,
                )
                Spacer(Modifier.size(6.dp))
                Text(
                    text = error.orEmpty(),
                    style = MaterialTheme.typography.bodySmall,
                    color = TextMuted,
                )
            }
        }

        if (controls) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .align(Alignment.TopCenter)
                    .background(
                        Brush.verticalGradient(
                            0f to Color.Black.copy(alpha = 0.75f),
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
                    Spacer(Modifier.width(8.dp))
                }
            }

            Row(
                modifier = Modifier.align(Alignment.Center),
                horizontalArrangement = Arrangement.spacedBy(28.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IconButton(
                    onClick = { exoPlayer.seekTo((exoPlayer.currentPosition - 10_000).coerceAtLeast(0)) },
                    modifier = Modifier.size(44.dp),
                ) {
                    Icon(Icons.Rounded.Replay10, contentDescription = "Back 10 seconds", tint = Color.White)
                }
                Box(
                    modifier = Modifier
                        .size(64.dp)
                        .clip(CircleShape)
                        .background(Color.Black.copy(alpha = 0.55f))
                        .clickable {
                            if (ended) {
                                exoPlayer.seekTo(0)
                                exoPlayer.play()
                                ended = false
                            } else if (playing) {
                                exoPlayer.pause()
                            } else {
                                exoPlayer.play()
                            }
                        },
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        imageVector = if (playing && !ended) {
                            Icons.Rounded.Pause
                        } else {
                            Icons.Rounded.PlayArrow
                        },
                        contentDescription = if (playing) "Pause" else "Play",
                        tint = Color.White,
                        modifier = Modifier.size(32.dp),
                    )
                }
                IconButton(
                    onClick = { exoPlayer.seekTo(exoPlayer.currentPosition + 10_000) },
                    modifier = Modifier.size(44.dp),
                ) {
                    Icon(Icons.Rounded.Forward10, contentDescription = "Forward 10 seconds", tint = Color.White)
                }
            }

            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .align(Alignment.BottomCenter)
                    .background(
                        Brush.verticalGradient(
                            0f to Color.Transparent,
                            1f to Color.Black.copy(alpha = 0.85f),
                        ),
                    )
                    .navigationBarsPadding()
                    .padding(horizontal = 18.dp, vertical = 10.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = formatTime(if (scrubbing) scrubValue.toLong() else position),
                        style = MaterialTheme.typography.labelSmall,
                        color = Color.White,
                    )
                    Slider(
                        value = if (scrubbing) {
                            scrubValue
                        } else {
                            position.toFloat().coerceIn(0f, duration.coerceAtLeast(1).toFloat())
                        },
                        onValueChange = { value ->
                            scrubbing = true
                            scrubValue = value
                        },
                        onValueChangeFinished = {
                            exoPlayer.seekTo(scrubValue.toLong())
                            position = scrubValue.toLong()
                            scrubbing = false
                        },
                        valueRange = 0f..duration.coerceAtLeast(1).toFloat(),
                        modifier = Modifier.weight(1f).padding(horizontal = 10.dp),
                        colors = SliderDefaults.colors(
                            thumbColor = Color.White,
                            activeTrackColor = Accent,
                            inactiveTrackColor = Color.White.copy(alpha = 0.25f),
                        ),
                    )
                    Text(
                        text = formatTime(duration),
                        style = MaterialTheme.typography.labelSmall,
                        color = Color.White,
                    )
                }
                Row(
                    modifier = Modifier.fillMaxWidth().padding(top = 2.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    Text(
                        text = "${formatSpeed(speed)}",
                        style = MaterialTheme.typography.labelMedium,
                        color = Color.White,
                        modifier = Modifier
                            .clip(RoundedCornerShape(50))
                            .background(Color.White.copy(alpha = 0.14f))
                            .clickable {
                                speed = nextSpeed(speed)
                                exoPlayer.setPlaybackSpeed(speed)
                            }
                            .padding(horizontal = 12.dp, vertical = 6.dp),
                    )
                    Text(
                        text = if (zoom) "Zoom" else "Fit",
                        style = MaterialTheme.typography.labelMedium,
                        color = Color.White,
                        modifier = Modifier
                            .clip(RoundedCornerShape(50))
                            .background(Color.White.copy(alpha = 0.14f))
                            .clickable { zoom = !zoom }
                            .padding(horizontal = 12.dp, vertical = 6.dp),
                    )
                    Spacer(Modifier.weight(1f))
                    if (playback.hasNext) {
                        Row(
                            modifier = Modifier
                                .clip(RoundedCornerShape(50))
                                .background(Color.White.copy(alpha = 0.14f))
                                .clickable { onNext() }
                                .padding(horizontal = 12.dp, vertical = 6.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Icon(
                                Icons.Rounded.SkipNext,
                                contentDescription = "Next episode",
                                tint = Color.White,
                                modifier = Modifier.size(16.dp),
                            )
                            Spacer(Modifier.width(4.dp))
                            Text(
                                text = "Next",
                                style = MaterialTheme.typography.labelMedium,
                                color = Color.White,
                            )
                        }
                    }
                }
            }
        }
    }
}

private fun formatTime(milliseconds: Long): String {
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

private fun formatSpeed(speed: Float): String =
    if (speed == speed.toInt().toFloat()) {
        "${speed.toInt()}x"
    } else {
        "${speed}x"
    }

private fun nextSpeed(speed: Float): Float = when (speed) {
    0.75f -> 1f
    1f -> 1.25f
    1.25f -> 1.5f
    1.5f -> 2f
    else -> 0.75f
}