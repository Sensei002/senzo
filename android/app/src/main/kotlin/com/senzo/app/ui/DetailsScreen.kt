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
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import coil.compose.AsyncImage
import com.senzo.app.DetailsState
import com.senzo.app.StreamItem
import com.senzo.app.Video

/**
 * The title page: backdrop, actions, description, and for a series the season
 * picker with its episodes. Playing opens the stream list of the addons.
 */
@Composable
fun DetailsScreen(
    state: DetailsState,
    onBack: () -> Unit,
    onPlayBest: () -> Unit,
    onPlayStream: (StreamItem) -> Unit,
    onSelectSeason: (Int) -> Unit,
    onSelectVideo: (Video) -> Unit,
    onToggleLibrary: () -> Unit,
    onRetry: () -> Unit,
    onPickerShown: () -> Unit,
) {
    var showStreams by remember(state.id) { mutableStateOf(false) }
    var expanded by remember(state.id) { mutableStateOf(false) }
    val meta = state.meta
    // The board's play button opens straight into the picker.
    LaunchedEffect(state.pickSource) {
        if (state.pickSource) {
            showStreams = true
            onPickerShown()
        }
    }
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
                onChooseStream = { showStreams = true },
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
                    LoadingBlock("Loading details…")
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
                        showStreams = true
                    },
                )
            }
        } else if (meta != null && meta.type == "movie") {
            item(key = "movie-meta") { MovieFacts(state = state) }
        }
    }

    if (showStreams) {
        StreamSheet(
            title = state.title,
            subtitle = state.selectedVideo?.label.orEmpty(),
            streams = state.streams,
            loading = state.streamsLoading,
            error = state.streamError,
            onPlay = { item ->
                showStreams = false
                onPlayStream(item)
            },
            onPlayBest = {
                showStreams = false
                onPlayBest()
            },
            onRetry = onRetry,
            onDismiss = { showStreams = false },
        )
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
                    LabelPill("★ $it", color = Color.White.copy(alpha = 0.16f))
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
                text = "Director · " + meta.director.joinToString(", "),
                style = MaterialTheme.typography.bodySmall,
                color = TextDim,
            )
        }
        if (meta.cast.isNotEmpty()) {
            Spacer(Modifier.height(4.dp))
            Text(
                text = "Cast · " + meta.cast.take(6).joinToString(", "),
                style = MaterialTheme.typography.bodySmall,
                color = TextDim,
            )
        }
    }
}

/** Which sources the sheet shows. */
private const val FILTER_ALL = "All"
private const val FILTER_TORRENT = "Torrent"
private const val FILTER_DIRECT = "Direct"
private val SOURCE_FILTERS = listOf(FILTER_ALL, FILTER_TORRENT, FILTER_DIRECT)

/**
 * The sources of the selected title.
 *
 * Every addon and every enabled plugin answers here, so the list is grouped by
 * kind and can be ordered by quality or by swarm size before anything starts
 * playing. A torrent entry goes through the local engine, a direct one through
 * the player as-is.
 */
@Composable
private fun StreamSheet(
    title: String,
    subtitle: String,
    streams: List<StreamItem>,
    loading: Boolean,
    error: String?,
    onPlay: (StreamItem) -> Unit,
    onPlayBest: () -> Unit,
    onRetry: () -> Unit,
    onDismiss: () -> Unit,
) {
    var filter by remember { mutableStateOf(FILTER_ALL) }
    var sortBySeeds by remember { mutableStateOf(false) }
    val torrents = streams.count { it.torrent != null }
    val direct = streams.size - torrents
    val filtered = streams.filter { item ->
        when (filter) {
            FILTER_TORRENT -> item.torrent != null
            FILTER_DIRECT -> item.torrent == null
            else -> true
        }
    }
    val ordered = if (sortBySeeds) {
        filtered.sortedByDescending { it.seeds ?: 0 }
    } else {
        filtered.sortedWith(
            compareByDescending<StreamItem> { it.qualityRank }.thenByDescending { it.seeds ?: 0 },
        )
    }

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.BottomCenter) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(max = 620.dp)
                    .clip(RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp))
                    .background(Surface)
                    .padding(top = 10.dp, bottom = 14.dp),
            ) {
                Box(
                    Modifier
                        .align(Alignment.CenterHorizontally)
                        .size(width = 40.dp, height = 4.dp)
                        .clip(CircleShape)
                        .background(Outline),
                )
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(start = 20.dp, end = 10.dp, top = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(Modifier.weight(1f)) {
                        Text(
                            text = "Choose a source",
                            style = MaterialTheme.typography.titleMedium,
                            color = TextPrimary,
                        )
                        Text(
                            text = listOfNotNull(title, subtitle.takeIf { it.isNotBlank() })
                                .joinToString(" · "),
                            style = MaterialTheme.typography.bodySmall,
                            color = TextDim,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        if (streams.isNotEmpty()) {
                            Text(
                                text = "$torrents torrents · $direct direct",
                                style = MaterialTheme.typography.labelSmall,
                                color = TextMuted,
                            )
                        }
                    }
                    if (streams.isNotEmpty()) {
                        TextButton(onClick = onPlayBest) {
                            Icon(
                                Icons.Rounded.PlayArrow,
                                contentDescription = null,
                                modifier = Modifier.size(16.dp),
                                tint = Accent,
                            )
                            Spacer(Modifier.width(2.dp))
                            Text("Best", color = Accent, style = MaterialTheme.typography.labelMedium)
                        }
                    }
                }

                if (streams.isNotEmpty()) {
                    Spacer(Modifier.height(8.dp))
                    LazyRow(
                        contentPadding = PaddingValues(horizontal = 20.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        // K2 crashes on enum `entries` here, so the three
                        // values are listed by hand.
                        items(SOURCE_FILTERS) { entry ->
                            val count = when (entry) {
                                FILTER_TORRENT -> torrents
                                FILTER_DIRECT -> direct
                                else -> streams.size
                            }
                            SourcePill(
                                text = "$entry $count",
                                selected = filter == entry,
                                onClick = { filter = entry },
                            )
                        }
                        item {
                            SourcePill(
                                text = if (sortBySeeds) "Sort: Seeds" else "Sort: Quality",
                                selected = sortBySeeds,
                                onClick = { sortBySeeds = !sortBySeeds },
                            )
                        }
                    }
                }

                Spacer(Modifier.height(4.dp))
                when {
                    loading && streams.isEmpty() -> Box(Modifier.fillMaxWidth().height(180.dp)) {
                        LoadingBlock("Asking your addons and plugins…")
                    }
                    error != null && streams.isEmpty() -> {
                        ErrorBlock(message = error, onRetry = onRetry)
                        TextButton(
                            onClick = onDismiss,
                            modifier = Modifier.align(Alignment.CenterHorizontally),
                        ) { Text("Close", color = TextMuted) }
                    }
                    streams.isEmpty() -> EmptyState(
                        title = "No sources",
                        body = "No addon or plugin offered a playable stream for this title.",
                    )
                    ordered.isEmpty() -> EmptyState(
                        title = "Nothing in this filter",
                        body = "Switch back to All to see every source.",
                    )
                    else -> LazyColumn(
                        modifier = Modifier.fillMaxWidth().weight(1f, fill = false),
                        contentPadding = PaddingValues(vertical = 6.dp),
                    ) {
                        items(ordered) { item ->
                            StreamRow(item = item, onPlay = { onPlay(item) })
                        }
                    }
                }
                if (loading && streams.isNotEmpty()) {
                    Text(
                        text = "Still searching for more sources…",
                        style = MaterialTheme.typography.labelSmall,
                        color = TextDim,
                        modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp),
                    )
                }
            }
        }
    }
}

@Composable
private fun SourcePill(text: String, selected: Boolean, onClick: () -> Unit) {
    Box(
        modifier = Modifier
            .height(32.dp)
            .clip(RoundedCornerShape(50))
            .background(if (selected) Accent.copy(alpha = 0.18f) else SurfaceHigh)
            .clickable(onClick = onClick)
            .padding(horizontal = 14.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = text,
            style = MaterialTheme.typography.labelSmall,
            color = if (selected) Accent else TextMuted,
        )
    }
}

@Composable
private fun StreamRow(item: StreamItem, onPlay: () -> Unit) {
    val torrent = item.torrent != null
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onPlay)
            .padding(horizontal = 20.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        LabelPill(
            text = item.badge,
            color = if (torrent) Gold.copy(alpha = 0.22f) else Accent.copy(alpha = 0.22f),
            contentColor = Color.White,
        )
        Column(Modifier.weight(1f)) {
            Text(
                text = item.addon.removePrefix("Plugin · "),
                style = MaterialTheme.typography.titleSmall,
                color = TextPrimary,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (item.detail.isNotBlank()) {
                Text(
                    text = item.detail,
                    style = MaterialTheme.typography.bodySmall,
                    color = TextDim,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Text(
                text = if (torrent) {
                    "Plays through the torrent engine"
                } else {
                    "Direct stream"
                },
                style = MaterialTheme.typography.labelSmall,
                color = TextDim,
            )
        }
        Column(horizontalAlignment = Alignment.End) {
            val caption = listOfNotNull(item.size, item.seeds?.let { "$it seeds" })
                .joinToString("\n")
            if (caption.isNotBlank()) {
                Text(
                    text = caption,
                    style = MaterialTheme.typography.labelSmall,
                    color = TextMuted,
                )
            }
            Spacer(Modifier.height(4.dp))
            Icon(
                Icons.Rounded.PlayArrow,
                contentDescription = null,
                tint = if (torrent) Gold else Accent,
                modifier = Modifier.size(18.dp),
            )
        }
    }
}
