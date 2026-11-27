package com.senzo.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
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
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.AspectRatio
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Cloud
import androidx.compose.material.icons.rounded.Forward10
import androidx.compose.material.icons.rounded.GraphicEq
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.LockOpen
import androidx.compose.material.icons.rounded.Pause
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Replay10
import androidx.compose.material.icons.rounded.SkipNext
import androidx.compose.material.icons.rounded.Speed
import androidx.compose.material.icons.rounded.Subtitles
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch

/**
 * The player's controls, drawn the way a phone player should look: the picture
 * stays untouched in the middle, transport sits over it, and every piece of
 * information lives along the edges where a thumb can reach it.
 *
 * Both engines paint this same chrome, so switching between Media3 and libmpv
 * never changes where a control is.
 */
@Composable
internal fun PlayerChrome(
    title: String,
    subtitle: String,
    playing: Boolean,
    ended: Boolean,
    position: Long,
    duration: Long,
    buffered: Long,
    zoom: Boolean,
    locked: Boolean,
    hasNext: Boolean,
    onPlayPause: () -> Unit,
    onSeekBy: (Long) -> Unit,
    onSeek: (Long) -> Unit,
    onZoom: () -> Unit,
    onSubtitles: () -> Unit,
    onAudio: () -> Unit,
    onSpeed: () -> Unit,
    onInfo: () -> Unit,
    onNext: () -> Unit,
    onRetry: () -> Unit,
    onLock: () -> Unit,
    onBack: () -> Unit,
) {
    if (locked) {
        Box(
            Modifier
                .fillMaxSize()
                .padding(horizontal = 20.dp)
                .navigationBarsPadding(),
            contentAlignment = Alignment.CenterStart,
        ) {
            ChromeIcon(Icons.Rounded.LockOpen, "Show controls", onClick = onLock)
        }
        return
    }

    var menu by remember { mutableStateOf(false) }
    Box(Modifier.fillMaxSize()) {
        Box(
            Modifier
                .align(Alignment.BottomCenter)
                .fillMaxWidth()
                .height(280.dp)
                .background(
                    Brush.verticalGradient(
                        0f to Color.Transparent,
                        1f to Color.Black.copy(alpha = 0.78f),
                    ),
                ),
        )
        Column(Modifier.fillMaxSize()) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .statusBarsPadding()
                    .padding(horizontal = 8.dp, vertical = 6.dp),
                horizontalArrangement = Arrangement.End,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                ChromeIcon(
                    icon = Icons.Rounded.AspectRatio,
                    description = if (zoom) "Fit the picture" else "Fill the screen",
                    tint = if (zoom) Accent else Color.White,
                    onClick = onZoom,
                )
                ChromeIcon(Icons.Rounded.Info, "Stream details", onClick = onInfo)
                ChromeIcon(Icons.Rounded.Lock, "Lock controls", onClick = onLock)
                ChromeIcon(Icons.AutoMirrored.Rounded.ArrowBack, "Close player", onClick = onBack)
            }

            Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    ChromeIcon(
                        icon = Icons.Rounded.Replay10,
                        description = "Back 10 seconds",
                        size = 38.dp,
                        onClick = { onSeekBy(-10_000) },
                    )
                    Spacer(Modifier.width(26.dp))
                    Box(
                        modifier = Modifier
                            .size(74.dp)
                            .clip(CircleShape)
                            .clickable(onClick = onPlayPause),
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(
                            imageVector = if (playing && !ended) Icons.Rounded.Pause else Icons.Rounded.PlayArrow,
                            contentDescription = if (playing) "Pause" else "Play",
                            tint = Color.White,
                            modifier = Modifier.size(46.dp),
                        )
                    }
                    Spacer(Modifier.width(26.dp))
                    ChromeIcon(
                        icon = Icons.Rounded.Forward10,
                        description = "Forward 10 seconds",
                        size = 38.dp,
                        onClick = { onSeekBy(10_000) },
                    )
                }
            }

            Column(
                Modifier
                    .fillMaxWidth()
                    .navigationBarsPadding()
                    .padding(horizontal = 22.dp, vertical = 8.dp),
            ) {
                Text(
                    text = title,
                    color = Color.White,
                    fontSize = 25.sp,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (subtitle.isNotBlank()) {
                    Spacer(Modifier.height(3.dp))
                    Text(
                        text = subtitle,
                        color = Color.White.copy(alpha = 0.68f),
                        fontSize = 13.sp,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Spacer(Modifier.height(10.dp))
                SeekBar(
                    position = position,
                    duration = duration,
                    buffered = buffered,
                    onSeek = onSeek,
                )
                Spacer(Modifier.height(4.dp))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    ChromeIcon(Icons.Rounded.Subtitles, "Subtitles", onClick = onSubtitles)
                    ChromeIcon(Icons.Rounded.GraphicEq, "Audio track", onClick = onAudio)
                    ChromeIcon(Icons.Rounded.Cloud, "Stream details", onClick = onInfo)
                    ChromeIcon(Icons.Rounded.Speed, "Playback speed", onClick = onSpeed)
                    if (hasNext) {
                        ChromeIcon(Icons.Rounded.SkipNext, "Next episode", onClick = onNext)
                    }
                    Box {
                        ChromeIcon(Icons.Rounded.ChevronRight, "More options") { menu = true }
                        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                            DropdownMenuItem(
                                text = { Text("Stream details") },
                                onClick = { menu = false; onInfo() },
                            )
                            if (hasNext) {
                                DropdownMenuItem(
                                    text = { Text("Next episode") },
                                    onClick = { menu = false; onNext() },
                                )
                            }
                            DropdownMenuItem(
                                text = { Text("Retry stream") },
                                onClick = { menu = false; onRetry() },
                            )
                            DropdownMenuItem(
                                text = { Text("Close player") },
                                onClick = { menu = false; onBack() },
                            )
                        }
                    }
                    Spacer(Modifier.weight(1f))
                    Text(
                        text = formatTime(position) + " / " + formatTime(duration),
                        color = Color.White,
                        fontSize = 14.sp,
                        fontWeight = FontWeight.Medium,
                    )
                }
            }
        }
    }
}

/** A quiet icon button: no ripple chrome, no filled circles over the picture. */
@Composable
private fun ChromeIcon(
    icon: ImageVector,
    description: String,
    size: Dp = 23.dp,
    tint: Color = Color.White,
    onClick: () -> Unit,
) {
    Box(
        Modifier.size(46.dp).clip(CircleShape).clickable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = description, tint = tint, modifier = Modifier.size(size))
    }
}

/**
 * A thin scrubber with a bar thumb, so the picture stays the loudest thing
 * on screen. Dragging previews the time and only seeks on release.
 */
@Composable
private fun SeekBar(
    position: Long,
    duration: Long,
    buffered: Long,
    onSeek: (Long) -> Unit,
) {
    var drag by remember { mutableStateOf<Float?>(null) }
    val live = if (duration > 0) (position.toFloat() / duration).coerceIn(0f, 1f) else 0f
    val shown = drag ?: live
    Box(
        Modifier
            .fillMaxWidth()
            .height(26.dp)
            .pointerInput(duration) {
                if (duration <= 0) return@pointerInput
                coroutineScope {
                    launch {
                        detectHorizontalDragGestures(
                            onDragStart = { offset ->
                                drag = (offset.x / size.width).coerceIn(0f, 1f)
                            },
                            onHorizontalDrag = { change, delta ->
                                change.consume()
                                drag = ((drag ?: 0f) + delta / size.width).coerceIn(0f, 1f)
                            },
                            onDragEnd = {
                                drag?.let { onSeek((it * duration).toLong()) }
                                drag = null
                            },
                            onDragCancel = { drag = null },
                        )
                    }
                    launch {
                        detectTapGestures { offset ->
                            onSeek(((offset.x / size.width).coerceIn(0f, 1f) * duration).toLong())
                        }
                    }
                }
            },
    ) {
        androidx.compose.foundation.Canvas(Modifier.fillMaxSize()) {
            val track = 2.5.dp.toPx()
            val radius = CornerRadius(track / 2)
            val middle = size.height / 2
            val bufferFraction = if (duration > 0) (buffered.toFloat() / duration).coerceIn(0f, 1f) else 0f
            drawRoundRect(
                color = Color.White.copy(alpha = 0.28f),
                topLeft = Offset(0f, middle - track / 2),
                size = Size(size.width, track),
                cornerRadius = radius,
            )
            drawRoundRect(
                color = Color.White.copy(alpha = 0.45f),
                topLeft = Offset(0f, middle - track / 2),
                size = Size(size.width * bufferFraction, track),
                cornerRadius = radius,
            )
            drawRoundRect(
                color = Color.White,
                topLeft = Offset(0f, middle - track / 2),
                size = Size(size.width * shown, track),
                cornerRadius = radius,
            )
            val thumbWidth = 3.dp.toPx()
            val thumbHeight = 13.dp.toPx()
            val left = (size.width * shown - thumbWidth / 2)
                .coerceIn(0f, (size.width - thumbWidth).coerceAtLeast(0f))
            drawRoundRect(
                color = Color.White,
                topLeft = Offset(left, middle - thumbHeight / 2),
                size = Size(thumbWidth, thumbHeight),
                cornerRadius = CornerRadius(thumbWidth / 2),
            )
        }
    }
}
