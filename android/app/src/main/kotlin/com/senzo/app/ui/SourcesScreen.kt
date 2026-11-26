package com.senzo.app.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
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
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Autorenew
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.Person
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.SearchOff
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.senzo.app.DetailsState
import com.senzo.app.RankedStream
import com.senzo.app.RankingMode
import com.senzo.app.StreamItem
import com.senzo.app.formatSize
import com.senzo.app.rankStreams

/**
 * The source page: every addon's answer in one list.
 *
 * The desktop client opens this page when a title is played, then groups the
 * replies by addon, ranks them, and lets the viewer change provider or ranking
 * without leaving the page. A spinner keeps turning while addons are still
 * answering, and the reload button starts the requests again.
 */
@Composable
fun SourcesScreen(
    state: DetailsState,
    ranking: String,
    showFiltered: Boolean,
    onBack: () -> Unit,
    onPlay: (StreamItem) -> Unit,
    onPlayBest: () -> Unit,
    onReload: () -> Unit,
    onRanking: (String) -> Unit,
    onShowFiltered: (Boolean) -> Unit,
) {
    BackHandler(onBack = onBack)
    val mode = RankingMode.of(ranking)
    val ranked = remember(state.streams, mode, showFiltered) {
        rankStreams(state.streams, mode, showFiltered)
    }
    val providers = remember(state.streams) {
        listOf("All") + state.streams.map { it.addon }.distinct()
    }
    var provider by remember(state.id) { mutableStateOf("All") }
    val visible = remember(ranked, provider) {
        if (provider == "All") ranked else ranked.filter { it.item.addon == provider }
    }
    val busy = state.streamsLoading || state.streamsPending > 0

    Column(Modifier.fillMaxSize().background(Background)) {
        SourcesHeader(
            state = state,
            busy = busy,
            onBack = onBack,
            onReload = onReload,
            onPlayBest = onPlayBest,
        )
        FilterRow(
            provider = provider,
            providers = providers,
            ranking = mode.label,
            onProvider = { provider = it },
            onRanking = onRanking,
        )
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(start = 18.dp, end = 18.dp, bottom = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Switch(
                checked = showFiltered,
                onCheckedChange = onShowFiltered,
                colors = SwitchDefaults.colors(
                    checkedThumbColor = OnAccent,
                    checkedTrackColor = Accent,
                ),
            )
            Spacer(Modifier.width(10.dp))
            Text(
                text = "Show suspicious or filtered sources",
                style = MaterialTheme.typography.bodySmall,
                color = TextMuted,
            )
        }

        when {
            visible.isEmpty() && busy -> LoadingSources()
            visible.isEmpty() && state.streamError != null -> ErrorBlock(
                message = state.streamError,
                modifier = Modifier.fillMaxWidth().padding(top = 40.dp),
                onRetry = onReload,
            )
            visible.isEmpty() -> EmptyState(
                title = "No sources answered",
                body = if (showFiltered) {
                    "No addon returned a stream for this video. Reload, or install more addons."
                } else {
                    "No usable stream was returned. Some results may be hidden as suspicious; turn the switch on to see them."
                },
                icon = Icons.Rounded.SearchOff,
                action = "Reload sources",
                onAction = onReload,
            )
            else -> SourceList(
                streams = visible,
                ranking = mode,
                busy = busy,
                onPlay = onPlay,
            )
        }
    }
}

@Composable
private fun SourcesHeader(
    state: DetailsState,
    busy: Boolean,
    onBack: () -> Unit,
    onReload: () -> Unit,
    onPlayBest: () -> Unit,
) {
    val spin = rememberInfiniteTransition(label = "sources")
    val angle by spin.animateFloat(
        initialValue = 0f,
        targetValue = 360f,
        animationSpec = infiniteRepeatable(tween(durationMillis = 900, easing = LinearEasing)),
        label = "sources-spin",
    )
    Column(
        Modifier
            .fillMaxWidth()
            .statusBarsPadding()
            .padding(start = 6.dp, end = 10.dp, top = 4.dp, bottom = 8.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onBack) {
                Icon(
                    Icons.AutoMirrored.Rounded.ArrowBack,
                    contentDescription = "Back to the title",
                    tint = TextPrimary,
                )
            }
            if (busy) {
                Icon(
                    imageVector = Icons.Rounded.Autorenew,
                    contentDescription = null,
                    tint = Accent,
                    modifier = Modifier.size(18.dp).rotate(angle),
                )
            } else {
                Box(
                    Modifier
                        .size(7.dp)
                        .clip(RoundedCornerShape(50))
                        .background(Accent),
                )
            }
            Spacer(Modifier.width(9.dp))
            Text(
                text = "SENZO SOURCES",
                color = TextPrimary,
                fontSize = 13.sp,
                fontWeight = FontWeight.ExtraBold,
                letterSpacing = 1.6.sp,
            )
            Spacer(Modifier.width(12.dp))
            Text(
                text = when {
                    busy && state.streams.isEmpty() -> "Loading sources…"
                    state.streamsPending > 0 -> "${state.streamsPending} addons still answering"
                    state.streams.isEmpty() -> "No sources yet"
                    state.streams.size == 1 -> "1 file"
                    else -> "${state.streams.size} files"
                },
                style = MaterialTheme.typography.labelSmall,
                color = TextMuted,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            if (!busy && state.streams.isNotEmpty()) {
                Box(
                    Modifier
                        .clip(RoundedCornerShape(50))
                        .background(Color.White.copy(alpha = 0.10f))
                        .clickable(onClick = onPlayBest)
                        .padding(horizontal = 12.dp, vertical = 6.dp),
                ) {
                    Text("Play best", color = TextPrimary, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
                }
            }
            IconButton(onClick = onReload) {
                Icon(Icons.Rounded.Refresh, contentDescription = "Reload sources", tint = TextMuted)
            }
        }
        if (state.selectedVideo != null) {
            Text(
                text = listOfNotNull(
                    state.selectedVideo.label,
                    state.selectedVideo.title?.takeIf { it.isNotBlank() },
                ).joinToString("  ·  "),
                style = MaterialTheme.typography.bodySmall,
                color = TextMuted,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.padding(start = 54.dp, end = 4.dp),
            )
        }
    }
}

@Composable
private fun FilterRow(
    provider: String,
    providers: List<String>,
    ranking: String,
    onProvider: (String) -> Unit,
    onRanking: (String) -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 18.dp, vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        FilterMenu(
            label = "Provider",
            value = provider,
            options = providers,
            onSelect = onProvider,
            modifier = Modifier.weight(1f),
        )
        FilterMenu(
            label = "Ranking",
            value = ranking,
            options = RankingMode.entries.map { it.label },
            onSelect = onRanking,
            modifier = Modifier.weight(0.9f),
        )
    }
}

@Composable
private fun FilterMenu(
    label: String,
    value: String,
    options: List<String>,
    onSelect: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    var open by remember { mutableStateOf(false) }
    Box(modifier) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(50))
                .background(SurfaceHigh)
                .clickable { open = true }
                .padding(horizontal = 14.dp, vertical = 9.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = label + "  ",
                style = MaterialTheme.typography.labelSmall,
                color = TextDim,
            )
            Text(
                text = value,
                style = MaterialTheme.typography.labelLarge,
                color = TextPrimary,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            Icon(
                Icons.Rounded.KeyboardArrowDown,
                contentDescription = null,
                tint = TextMuted,
                modifier = Modifier.size(18.dp),
            )
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            options.forEach { option ->
                DropdownMenuItem(
                    text = {
                        Text(
                            text = option,
                            color = if (option == value) Accent else TextPrimary,
                        )
                    },
                    onClick = {
                        open = false
                        onSelect(option)
                    },
                )
            }
        }
    }
}

@Composable
private fun SourceList(
    streams: List<RankedStream>,
    ranking: RankingMode,
    busy: Boolean,
    onPlay: (StreamItem) -> Unit,
) {
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(bottom = 28.dp),
    ) {
        itemsIndexed(streams, key = { index, _ -> index }) { index, ranked ->
            val previous = streams.getOrNull(index - 1)
            val newGroup = previous == null || previous.item.addon != ranked.item.addon
            Column(Modifier.fillMaxWidth()) {
                if (newGroup) {
                    val count = streams.count { it.item.addon == ranked.item.addon }
                    Text(
                        text = ranked.item.addon.uppercase() + "  ·  $count",
                        color = TextDim,
                        fontSize = 10.sp,
                        fontWeight = FontWeight.Bold,
                        letterSpacing = 1.2.sp,
                        modifier = Modifier.padding(start = 18.dp, end = 18.dp, top = 16.dp, bottom = 6.dp),
                    )
                }
                SourceRow(
                    ranked = ranked,
                    showScore = ranking == RankingMode.Smart,
                    onPlay = { onPlay(ranked.item) },
                )
                Box(Modifier.fillMaxWidth().height(1.dp).background(Outline.copy(alpha = 0.5f)))
            }
        }
        if (busy) {
            item {
                Row(
                    modifier = Modifier.fillMaxWidth().padding(18.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "Still waiting for addons…",
                        style = MaterialTheme.typography.labelSmall,
                        color = TextDim,
                    )
                }
            }
        }
    }
}

@Composable
private fun SourceRow(
    ranked: RankedStream,
    showScore: Boolean,
    onPlay: () -> Unit,
) {
    val item = ranked.item
    val torrent = item.torrent != null
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onPlay)
            .padding(horizontal = 18.dp, vertical = 13.dp),
    ) {
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                LabelPill(
                    text = item.badge,
                    color = if (torrent) Gold.copy(alpha = 0.18f) else Accent.copy(alpha = 0.18f),
                )
                Spacer(Modifier.width(8.dp))
                Text(
                    text = item.name,
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold,
                    color = TextPrimary,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                if (showScore && ranked.score != 0) {
                    Spacer(Modifier.width(8.dp))
                    Text(
                        text = (if (ranked.score > 0) "+" else "") + ranked.score,
                        style = MaterialTheme.typography.labelSmall,
                        color = TextDim,
                    )
                }
            }
            if (item.detail.isNotBlank()) {
                Spacer(Modifier.height(3.dp))
                Text(
                    text = item.detail,
                    style = MaterialTheme.typography.bodySmall,
                    color = TextMuted,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Spacer(Modifier.height(7.dp))
            Row(verticalAlignment = Alignment.CenterVertically) {
                val seeders = ranked.seeders
                Icon(
                    Icons.Rounded.Person,
                    contentDescription = null,
                    tint = when {
                        seeders == null -> TextDim
                        seeders >= 25 -> Color(0xFF69DB7C)
                        seeders >= 1 -> Gold
                        else -> Color(0xFFFF8787)
                    },
                    modifier = Modifier.size(13.dp),
                )
                Spacer(Modifier.width(4.dp))
                Text(
                    text = when {
                        seeders == null -> if (torrent) "seeders unknown" else "direct stream"
                        seeders == 0 -> "no seeders"
                        seeders == 1 -> "1 seeder"
                        else -> "$seeders seeders"
                    },
                    fontSize = 11.sp,
                    fontWeight = FontWeight.SemiBold,
                    color = when {
                        seeders == null -> TextDim
                        seeders >= 25 -> Color(0xFF69DB7C)
                        seeders >= 1 -> Gold
                        else -> Color(0xFFFF8787)
                    },
                )
                val sizeText = ranked.sizeBytes?.let { formatSize(it) } ?: item.size
                if (sizeText != null) {
                    Text("  ·  ", fontSize = 11.sp, color = TextDim)
                    Text(sizeText, fontSize = 11.sp, color = TextDim, fontWeight = FontWeight.Medium)
                }
                Text("  ·  ", fontSize = 11.sp, color = TextDim)
                Text(
                    text = if (torrent) "P2P" else "Direct",
                    fontSize = 11.sp,
                    color = TextDim,
                )
                if (ranked.filtered) {
                    Text("  ·  ", fontSize = 11.sp, color = TextDim)
                    Text("Filtered", fontSize = 11.sp, color = Gold)
                }
            }
        }
    }
}

@Composable
private fun LoadingSources() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            val spin = rememberInfiniteTransition(label = "loading")
            val angle by spin.animateFloat(
                initialValue = 0f,
                targetValue = 360f,
                animationSpec = infiniteRepeatable(tween(durationMillis = 900, easing = LinearEasing)),
                label = "loading-spin",
            )
            Icon(
                Icons.Rounded.Autorenew,
                contentDescription = null,
                tint = Accent,
                modifier = Modifier.size(30.dp).rotate(angle),
            )
            Spacer(Modifier.height(12.dp))
            Text(
                text = "Loading sources…",
                style = MaterialTheme.typography.bodySmall,
                color = TextMuted,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                text = "Asking every addon and plugin for a stream",
                style = MaterialTheme.typography.labelSmall,
                color = TextDim,
            )
        }
    }
}
