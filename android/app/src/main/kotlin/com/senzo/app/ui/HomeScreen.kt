package com.senzo.app.ui

import androidx.compose.foundation.Image
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
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.senzo.app.MetaPreview
import com.senzo.app.R
import com.senzo.app.UiState
import com.senzo.app.WatchProgress

/**
 * The board: a full-bleed hero of the first titles, the resume points, then one
 * row per catalogue the installed addons serve.
 *
 * The header floats over the artwork rather than sitting on a strip of its own,
 * so the first thing on screen is a poster and not chrome.
 */
@Composable
fun HomeScreen(
    state: UiState,
    onOpen: (MetaPreview) -> Unit,
    onPlay: (MetaPreview) -> Unit,
    onResume: (WatchProgress) -> Unit,
    onRefresh: () -> Unit,
    onSearch: () -> Unit,
    onProfile: () -> Unit,
    onForgetProgress: (String) -> Unit,
) {
    Box(Modifier.fillMaxSize().background(Background)) {
        when {
            state.boardLoading && state.board.isEmpty() ->
                LoadingBlock("Building your board…", Modifier.padding(top = 140.dp))

            state.board.isEmpty() && state.boardError != null ->
                ErrorBlock(
                    message = state.boardError,
                    modifier = Modifier.padding(top = 140.dp),
                    onRetry = onRefresh,
                )

            state.board.isEmpty() -> EmptyState(
                title = "Nothing on the board yet",
                body = "Install an addon from the Addons tab, then refresh.",
                modifier = Modifier.padding(top = 140.dp),
            )

            else -> Board(
                state = state,
                onOpen = onOpen,
                onPlay = onPlay,
                onResume = onResume,
                onForgetProgress = onForgetProgress,
            )
        }
        HomeTopBar(
            email = state.session?.email.orEmpty(),
            onSearch = onSearch,
            onRefresh = onRefresh,
            onProfile = onProfile,
        )
    }
}

@Composable
private fun Board(
    state: UiState,
    onOpen: (MetaPreview) -> Unit,
    onPlay: (MetaPreview) -> Unit,
    onResume: (WatchProgress) -> Unit,
    onForgetProgress: (String) -> Unit,
) {
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(bottom = 24.dp),
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
                    modifier = Modifier.padding(start = 18.dp, end = 8.dp, top = 18.dp),
                    action = "Clear",
                    onAction = { state.continueWatching.forEach { onForgetProgress(it.key) } },
                )
            }
            item(key = "continue-row") {
                LazyRow(
                    contentPadding = PaddingValues(horizontal = 18.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    items(state.continueWatching, key = { it.key }) { entry ->
                        ContinueCard(entry = entry, onClick = { onResume(entry) })
                    }
                }
            }
        }
        items(state.board, key = { it.title }) { row ->
            Column(Modifier.padding(top = 20.dp)) {
                SectionHeader(
                    title = row.title,
                    modifier = Modifier.padding(horizontal = 18.dp),
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

/** The bar that floats over the hero: the mark, search, refresh and the account. */
@Composable
private fun HomeTopBar(
    email: String,
    onSearch: () -> Unit,
    onRefresh: () -> Unit,
    onProfile: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(
                Brush.verticalGradient(
                    0f to Background.copy(alpha = 0.94f),
                    0.65f to Background.copy(alpha = 0.55f),
                    1f to Color.Transparent,
                ),
            )
            .statusBarsPadding()
            .padding(start = 16.dp, end = 6.dp, top = 8.dp, bottom = 22.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Image(
            painter = painterResource(R.drawable.senzo_mark),
            contentDescription = null,
            contentScale = ContentScale.Fit,
            modifier = Modifier.height(24.dp),
        )
        Spacer(Modifier.size(8.dp))
        Text(
            text = "SENZO",
            style = SenzoWordmark.copy(fontSize = MaterialTheme.typography.titleMedium.fontSize),
            color = TextPrimary,
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
                .size(32.dp)
                .clip(CircleShape)
                .background(Accent.copy(alpha = 0.18f))
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
        Spacer(Modifier.size(10.dp))
    }
}
