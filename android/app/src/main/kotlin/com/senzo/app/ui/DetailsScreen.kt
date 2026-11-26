package com.senzo.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import coil.compose.AsyncImage
import coil.request.ImageRequest
import com.senzo.app.DetailsState
import com.senzo.app.StreamItem
import com.senzo.app.Video

@Composable
fun DetailsScreen(
    state: DetailsState,
    onBack: () -> Unit,
    onSelectVideo: (String) -> Unit,
    onPlay: (StreamItem) -> Unit,
) {
    val meta = state.meta
    Column(Modifier.fillMaxSize().background(Background).statusBarsPadding()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = onBack) {
                Icon(Icons.Filled.ArrowBack, contentDescription = "Back", tint = TextPrimary)
            }
            Text(
                text = meta?.name ?: "",
                style = MaterialTheme.typography.titleMedium,
                color = TextPrimary,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }

        when {
            state.loading -> LoadingBlock("Loading details…")
            meta == null -> MessageBlock(
                message = "No addon returned metadata for this title.",
                actionLabel = "Back",
                onAction = onBack,
            )
            else -> DetailsBody(
                state = state,
                onSelectVideo = onSelectVideo,
                onPlay = onPlay,
            )
        }
    }
}

@Composable
private fun DetailsBody(
    state: DetailsState,
    onSelectVideo: (String) -> Unit,
    onPlay: (StreamItem) -> Unit,
) {
    val meta = state.meta ?: return
    val context = LocalContext.current
    val episodes = meta.videos.filter { it.id.isNotBlank() }
    val seasons = episodes.mapNotNull { it.season }.distinct().sorted()
    var season by rememberSaveable(meta.id) {
        mutableStateOf(seasons.firstOrNull() ?: 0)
    }

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(bottom = 32.dp),
    ) {
        item {
            Box(Modifier.fillMaxWidth().height(280.dp)) {
                AsyncImage(
                    model = ImageRequest.Builder(context)
                        .data(meta.background ?: meta.poster)
                        .crossfade(true)
                        .build(),
                    contentDescription = null,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.fillMaxSize(),
                )
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .background(
                            androidx.compose.ui.graphics.Brush.verticalGradient(
                                listOf(
                                    Background.copy(alpha = 0.2f),
                                    Background.copy(alpha = 0.75f),
                                    Background,
                                ),
                            ),
                        ),
                )
                Column(
                    modifier = Modifier
                        .align(Alignment.BottomStart)
                        .padding(16.dp),
                ) {
                    Text(
                        text = meta.name,
                        style = MaterialTheme.typography.headlineMedium,
                        color = TextPrimary,
                    )
                    Spacer(Modifier.height(6.dp))
                    TagRow(
                        tags = listOfNotNull(
                            meta.releaseInfo,
                            meta.imdbRating?.let { "★ $it" },
                            meta.runtime,
                            *meta.genres.take(2).toTypedArray(),
                        ),
                        modifier = Modifier.padding(0.dp),
                    )
                }
                PlayBadge(
                    hasStream = state.streams.isNotEmpty(),
                    onClick = { state.streams.firstOrNull()?.let(onPlay) },
                    modifier = Modifier.align(Alignment.CenterEnd).padding(16.dp),
                )
            }
        }

        if (!meta.description.isNullOrBlank()) {
            item {
                Text(
                    text = meta.description,
                    style = MaterialTheme.typography.bodyMedium,
                    color = TextMuted,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
            }
        }

        if (seasons.isNotEmpty()) {
            item {
                LazyRow(
                    contentPadding = PaddingValues(horizontal = 16.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(seasons) { candidate ->
                        FilterChip(
                            selected = candidate == season,
                            onClick = { season = candidate },
                            label = { Text("Season $candidate") },
                        )
                    }
                }
            }
            items(
                episodes.filter { it.season == season },
                key = { it.id },
            ) { episode ->
                EpisodeRow(
                    episode = episode,
                    selected = state.selectedVideo == episode.id,
                    onClick = { onSelectVideo(episode.id) },
                )
            }
        }

        item {
            SectionTitle(
                text = if (state.streamsLoading) "Streams" else "Streams (${state.streams.size})",
            )
        }

        when {
            state.streamsLoading -> item { LoadingBlock("Asking your addons for streams…") }
            state.streamError != null -> item {
                MessageBlock(message = state.streamError)
            }
            else -> items(state.streams) { stream ->
                StreamRow(stream = stream, onClick = { onPlay(stream) })
            }
        }
    }
}

@Composable
private fun PlayBadge(
    hasStream: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier = modifier
            .size(64.dp)
            .clip(RoundedCornerShape(50))
            .background(if (hasStream) Accent else SurfaceHigh)
            .clickable(enabled = hasStream, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            Icons.Filled.PlayArrow,
            contentDescription = "Play",
            tint = TextPrimary,
            modifier = Modifier.size(36.dp),
        )
    }
}

@Composable
private fun EpisodeRow(episode: Video, selected: Boolean, onClick: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .background(if (selected) SurfaceHigh else Background)
            .padding(horizontal = 16.dp, vertical = 10.dp),
    ) {
        Text(
            text = episode.label,
            style = MaterialTheme.typography.bodyMedium,
            fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal,
            color = if (selected) TextPrimary else TextMuted,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun StreamRow(stream: StreamItem, onClick: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Icon(
            Icons.Filled.PlayArrow,
            contentDescription = null,
            tint = Accent,
            modifier = Modifier.size(22.dp),
        )
        Column(Modifier.weight(1f)) {
            Text(
                text = stream.name,
                style = MaterialTheme.typography.bodyMedium,
                color = TextPrimary,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (stream.detail.isNotBlank()) {
                Text(
                    text = stream.detail,
                    style = MaterialTheme.typography.bodySmall,
                    color = TextMuted,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        Text(
            text = stream.addon,
            style = MaterialTheme.typography.bodySmall,
            color = TextMuted.copy(alpha = 0.7f),
            maxLines = 1,
        )
    }
}
