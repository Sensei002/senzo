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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.senzo.app.MetaPreview
import com.senzo.app.Tab
import com.senzo.app.UiState

/**
 * The board: a hero of the first titles, the resume points, then one row per
 * catalogue the installed addons serve.
 */
@Composable
fun HomeScreen(
    state: UiState,
    onOpen: (MetaPreview) -> Unit,
    onPlay: (MetaPreview) -> Unit,
    onResume: (com.senzo.app.WatchProgress) -> Unit,
    onRefresh: () -> Unit,
    onSearch: () -> Unit,
    onProfile: () -> Unit,
    onForgetProgress: (String) -> Unit,
) {
    Column(Modifier.fillMaxSize().background(Background)) {
        HomeHeader(
            email = state.session?.email.orEmpty(),
            onSearch = onSearch,
            onRefresh = onRefresh,
            onProfile = onProfile,
        )
        when {
            state.boardLoading && state.board.isEmpty() -> LoadingBlock("Building your board…")
            state.board.isEmpty() && state.boardError != null -> ErrorBlock(
                message = state.boardError,
                modifier = Modifier.weight(1f),
                onRetry = onRefresh,
            )
            state.board.isEmpty() -> EmptyState(
                title = "Nothing on the board yet",
                body = "Install an addon from the Addons tab, then refresh.",
                modifier = Modifier.weight(1f),
            )
            else -> LazyColumn(
                modifier = Modifier.weight(1f),
                contentPadding = PaddingValues(bottom = 28.dp),
            ) {
                if (state.hero.isNotEmpty()) {
                    item(key = "hero") {
                        HeroCarousel(
                            items = state.hero,
                            onOpen = onOpen,
                            onPlay = onPlay,
                        )
                    }
                }
                if (state.continueWatching.isNotEmpty()) {
                    item(key = "continue-header") {
                        SectionHeader(
                            title = "Continue watching",
                            modifier = Modifier.padding(start = 20.dp, end = 12.dp, top = 8.dp),
                            action = "Clear",
                            onAction = {
                                state.continueWatching.forEach { onForgetProgress(it.key) }
                            },
                        )
                    }
                    item(key = "continue-row") {
                        LazyRow(
                            contentPadding = PaddingValues(horizontal = 20.dp),
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            items(state.continueWatching, key = { it.key }) { entry ->
                                ContinueCard(
                                    entry = entry,
                                    onClick = { onResume(entry) },
                                )
                            }
                        }
                    }
                }
                items(state.board, key = { it.title }) { row ->
                    Column(Modifier.padding(top = 22.dp)) {
                        SectionHeader(
                            title = row.title,
                            modifier = Modifier.padding(horizontal = 20.dp),
                        )
                        Spacer(Modifier.height(10.dp))
                        PosterRow(
                            items = row.items,
                            progressFor = { item ->
                                state.progress
                                    .firstOrNull {
                                        it.type == item.type && it.metaId == item.id && !it.finished
                                    }
                                    ?.fraction
                            },
                            onOpen = onOpen,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun HomeHeader(
    email: String,
    onSearch: () -> Unit,
    onRefresh: () -> Unit,
    onProfile: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(Background)
            .statusBarsPadding()
            .padding(start = 20.dp, end = 8.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = "SENZO",
            style = SenzoWordmark,
            color = Accent,
        )
        Spacer(Modifier.weight(1f))
        IconButton(onClick = onSearch) {
            Icon(Icons.Rounded.Search, contentDescription = "Search", tint = TextPrimary)
        }
        IconButton(onClick = onRefresh) {
            Icon(Icons.Rounded.Refresh, contentDescription = "Refresh board", tint = TextPrimary)
        }
        Box(
            modifier = Modifier
                .size(34.dp)
                .clip(CircleShape)
                .background(Accent.copy(alpha = 0.22f))
                .clickable(onClick = onProfile),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                text = email.take(1).uppercase().ifBlank { "S" },
                style = MaterialTheme.typography.labelLarge,
                color = Accent,
                textAlign = TextAlign.Center,
            )
        }
        Spacer(Modifier.size(8.dp))
    }
}