package com.senzo.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Bookmark
import androidx.compose.material.icons.rounded.BookmarkBorder
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
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
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import coil.compose.AsyncImage
import com.senzo.app.DetailsState
import com.senzo.app.Video

/**
 * The title page: backdrop, actions, description, and for a series the season
 * picker with its episodes. Playing opens the stream list of the addons.
 */
@Composable
fun DetailsScreen(
    state: DetailsState,
    onBack: () -> Unit,
    onOpenSources: () -> Unit,
    onSelectSeason: (Int) -> Unit,
    onSelectVideo: (Video) -> Unit,
    onToggleLibrary: () -> Unit,
    onRetry: () -> Unit,
) {
    var expanded by remember(state.id) { mutableStateOf(false) }
    val meta = state.meta
    val seasons = meta?.seasons.orEmpty()
    val episodes = meta?.episodes(state.season ?: seasons.firstOrNull() ?: 0).orEmpty()

    LazyColumn(
        modifier = Modifier.fillMaxSize().background(Background),
        contentPadding = PaddingValues(bottom = 32.dp),
    ) {
        item(key = "backdrop") {
            Backdrop(state = state, onBack = onBack)
        }
        item(key = "actions") {
            ActionsRow(
                state = state,
                onChooseStream = onOpenSources,
                onToggleLibrary = onToggleLibrary,
                onRetry = onRetry,
            )
        }
        meta?.description?.takeIf { it.isNotBlank() }?.let { description ->
            item(key = "description") {
                Column(Modifier.padding(horizontal = 20.dp, vertical = 12.dp)) {
                    Text(
                        text = description,
                        style = MaterialTheme.typography.bodyMedium,
                        color = TextMuted,
                        maxLines = if (expanded) 40 else 4,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        text = if (expanded) "Show less" else "Read more",
                        style = MaterialTheme.typography.labelMedium,
                        color = Accent,
                        modifier = Modifier
                            .padding(top = 6.dp)
                            .clip(RoundedCornerShape(50))
                            .clickable { expanded = !expanded }
                            .padding(vertical = 4.dp),
                    )
                }
            }
        }
        if (state.loading && meta == null) {
            item(key = "loading") {
                Box(Modifier.fillMaxWidth().height(160.dp)) {
                    LoadingBlock("Loading detailsâ€¦")
                }
            }
        }
        if (seasons.isNotEmpty()) {
            item(key = "seasons") {
                Column {
                    SectionHeader(
                        title = "Episodes",
                        modifier = Modifier.padding(start = 20.dp, end = 20.dp),
                    )
                    Spacer(Modifier.height(8.dp))
                    LazyRow(
                        contentPadding = PaddingValues(horizontal = 20.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        items(seasons, key = { it }) { season ->
                            val selected = season == (state.season ?: seasons.first())
                            Text(
                                text = "Season $season",
                                style = MaterialTheme.typography.labelMedium,
                                color = if (selected) Color.White else TextMuted,
                                modifier = Modifier
                                    .clip(RoundedCornerShape(50))
                                    .background(if (selected) Accent else Surface)
                                    .clickable { onSelectSeason(season) }
                                    .padding(horizontal = 14.dp, vertical = 8.dp),
                            )
                        }
                    }
                }
            }
            items(episodes, key = { it.id }) { episode ->
                EpisodeRow(
                    video = episode,
                    selected = state.selectedVideo?.id == episode.id,
                    progress = state.resume
                        ?.takeIf { it.videoId == episode.id }
                        ?.fraction,
                    onClick = {
                        onSelectVideo(episode)
                        onOpenSources()
                    },
                )
            }
        } else if (meta != null && meta.type == "movie") {
            item(key = "movie-meta") { MovieFacts(state = state) }
        }
    }
}

@Composable
private fun Backdrop(state: DetailsState, onBack: () -> Unit) {
    Box(Modifier.fillMaxWidth().height(330.dp)) {
        AsyncImage(
            model = state.backdrop,
            contentDescription = state.title,
            contentScale = ContentScale.Crop,
            modifier = Modifier.fillMaxSize(),
        )
        Box(
            Modifier
                .fillMaxSize()
                .background(
                    Brush.verticalGradient(
                        0f to Color.Black.copy(alpha = 0.45f),
                        0.45f to Background.copy(alpha = 0.25f),
                        1f to Background,
                    ),
                ),
        )
        IconButton(
            onClick = onBack,
            modifier = Modifier
                .statusBarsPadding()
                .padding(start = 6.dp, top = 6.dp)
                .size(38.dp)
                .clip(CircleShape)
                .background(Color.Black.copy(alpha = 0.45f)),
        ) {
            Icon(Icons.Rounded.ArrowBack, contentDescription = "Back", tint = Color.White)
        }
        Column(
            modifier = Modifier
                .align(Alignment.BottomStart)
                .padding(horizontal = 20.dp),
        ) {
            Text(
                text = state.title,
                style = MaterialTheme.typography.headlineMedium,
                color = Color.White,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Spacer(Modifier.height(8.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                val meta = state.meta
                meta?.releaseInfo?.let { info ->
                    val year = Regex("\\b(19|20)\\d{2}\\b").find(info)?.value
                    if (year != null) {
                        LabelPill(year, color = Color.White.copy(alpha = 0.16f))
                    }
                }
                meta?.imdbRating?.takeIf { it.isNotBlank() }?.let {
                    LabelPill("â˜… $it", color = Color.White.copy(alpha = 0.16f))
                }
                meta?.runtime?.takeIf { it.isNotBlank() }?.let {
                    LabelPill(it, color = Color.White.copy(alpha = 0.16f))
                }
                LabelPill(
                    text = if (state.type == "series") "Series" else "Movie",
                    color = Accent.copy(alpha = 0.30f),
                )
            }
        }
    }
}

@Composable
private fun ActionsRow(
    state: DetailsState,
    onChooseStream: () -> Unit,
    onToggleLibrary: () -> Unit,
    onRetry: () -> Unit,
) {
    val resume = state.resume
    Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Button(
                onClick = onChooseStream,
                enabled = state.streams.isNotEmpty() || state.streamsLoading || state.streamError != null,
                shape = RoundedCornerShape(50),
                colors = ButtonDefaults.buttonColors(
                    containerColor = Color.White,
                    contentColor = Color.Black,
                    disabledContainerColor = SurfaceHigh,
                    disabledContentColor = TextDim,
                ),
                contentPadding = PaddingValues(horizontal = 22.dp, vertical = 12.dp),
            ) {
                Icon(
                    Icons.Rounded.PlayArrow,
                    contentDescription = null,
                    modifier = Modifier.size(18.dp),
                )
                Spacer(Modifier.width(6.dp))
                Text(
                    text = when {
                        state.streamsLoading -> "Finding sources"
                        resume != null && !resume.finished -> "Resume"
                        else -> "Play"
                    },
                    style = MaterialTheme.typography.labelLarge,
                )
            }
            Spacer(Modifier.width(10.dp))
            if (state.streams.isNotEmpty()) {
                TextButton(onClick = onChooseStream) {
                    Text(
                        text = "${state.streams.size} sources",
                        style = MaterialTheme.typography.labelMedium,
                        color = TextMuted,
                    )
                }
            }
            Spacer(Modifier.weight(1f))
            IconButton(onClick = onToggleLibrary) {
                Icon(
                    imageVector = if (state.inLibrary) {
                        Icons.Rounded.Bookmark
                    } else {
                        Icons.Rounded.BookmarkBorder
                    },
                    contentDescription = if (state.inLibrary) "Remove from library" else "Save to library",
                    tint = if (state.inLibrary) Accent else TextPrimary,
                )
            }
        }
        if (state.streamError != null && state.streams.isEmpty()) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = state.streamError,
                    style = MaterialTheme.typography.bodySmall,
                    color = TextDim,
                    modifier = Modifier.weight(1f),
                )
                TextButton(onClick = onRetry) { Text("Retry", color = Accent) }
            }
        }
        if (resume != null && !resume.finished && resume.durationMs > 0) {
            Text(
                text = "${(resume.fraction * 100).toInt()}% watched",
                style = MaterialTheme.typography.bodySmall,
                color = TextDim,
                modifier = Modifier.padding(top = 8.dp),
            )
        }
    }
}

@Composable
private fun EpisodeRow(
    video: Video,
    selected: Boolean,
    progress: Float?,
    onClick: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 20.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Box(
            modifier = Modifier
                .width(124.dp)
                .aspectRatio(16f / 9f)
                .clip(RoundedCornerShape(10.dp))
                .background(SurfaceHigh),
        ) {
            AsyncImage(
                model = video.thumbnail,
                contentDescription = video.title,
                contentScale = ContentScale.Crop,
                modifier = Modifier.fillMaxSize(),
            )
            if (progress != null && progress > 0f) {
                Box(
                    Modifier
                        .align(Alignment.BottomStart)
                        .padding(4.dp)
                        .height(3.dp)
                        .fillMaxWidth(progress)
                        .clip(CircleShape)
                        .background(Accent),
                )
            }
        }
        Column(Modifier.weight(1f)) {
            Text(
                text = video.label,
                style = MaterialTheme.typography.titleSmall,
                color = if (selected) Accent else TextPrimary,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            video.overview?.takeIf { it.isNotBlank() }?.let { overview ->
                Spacer(Modifier.height(3.dp))
                Text(
                    text = overview,
                    style = MaterialTheme.typography.bodySmall,
                    color = TextDim,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        if (selected) ActiveMark()
    }
}

@Composable
private fun MovieFacts(state: DetailsState) {
    val meta = state.meta ?: return
    Column(Modifier.padding(horizontal = 20.dp, vertical = 10.dp)) {
        if (meta.genres.isNotEmpty()) {
            LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                items(meta.genres.take(8), key = { it }) { genre ->
                    LabelPill(genre)
                }
            }
            Spacer(Modifier.height(10.dp))
        }
        if (meta.director.isNotEmpty()) {
            Text(
                text = "Director Â· " + meta.director.joinToString(", "),
                style = MaterialTheme.typography.bodySmall,
                color = TextDim,
            )
        }
        if (meta.cast.isNotEmpty()) {
            Spacer(Modifier.height(4.dp))
            Text(
                text = "Cast Â· " + meta.cast.take(6).joinToString(", "),
                style = MaterialTheme.typography.bodySmall,
                color = TextDim,
            )
        }
    }
}

