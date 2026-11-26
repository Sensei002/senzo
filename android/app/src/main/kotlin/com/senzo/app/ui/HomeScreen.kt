package com.senzo.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items as gridItems
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Subscriptions
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.senzo.app.AddonDescriptor
import com.senzo.app.CatalogRow
import com.senzo.app.MetaPreview
import com.senzo.app.UiState

private enum class HomeTab(val label: String) {
    Board("Board"),
    Addons("Addons"),
    Account("Account"),
}

/**
 * The board of the account: the catalogues of every installed addon, with the
 * same search that sits in the desktop header.
 */
@Composable
fun HomeScreen(
    state: UiState,
    onSearch: (String) -> Unit,
    onClearSearch: () -> Unit,
    onOpen: (String, String) -> Unit,
    onRefresh: () -> Unit,
    onSignOut: () -> Unit,
    onDismissMessage: () -> Unit,
) {
    var tab by rememberSaveable { mutableStateOf(HomeTab.Board.name) }
    val active = HomeTab.valueOf(tab)
    var searchVisible by rememberSaveable { mutableStateOf(false) }

    Scaffold(
        containerColor = Background,
        bottomBar = {
            NavigationBar(containerColor = Surface) {
                NavigationBarItem(
                    selected = active == HomeTab.Board,
                    onClick = { tab = HomeTab.Board.name },
                    icon = { Icon(Icons.Filled.Subscriptions, contentDescription = null) },
                    label = { Text(HomeTab.Board.label) },
                )
                NavigationBarItem(
                    selected = active == HomeTab.Addons,
                    onClick = { tab = HomeTab.Addons.name },
                    icon = { Icon(Icons.Filled.Subscriptions, contentDescription = null) },
                    label = { Text(HomeTab.Addons.label) },
                )
                NavigationBarItem(
                    selected = active == HomeTab.Account,
                    onClick = { tab = HomeTab.Account.name },
                    icon = { Icon(Icons.Filled.Person, contentDescription = null) },
                    label = { Text(HomeTab.Account.label) },
                )
            }
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .background(Background),
        ) {
            when (active) {
                HomeTab.Board -> BoardTab(
                    state = state,
                    searchVisible = searchVisible,
                    onToggleSearch = {
                        searchVisible = !searchVisible
                        if (!searchVisible) onClearSearch()
                    },
                    onSearch = onSearch,
                    onClearSearch = onClearSearch,
                    onOpen = onOpen,
                    onRefresh = onRefresh,
                )
                HomeTab.Addons -> AddonsTab(
                    addons = state.addons,
                    onRefresh = onRefresh,
                )
                HomeTab.Account -> AccountTab(
                    email = state.session?.email.orEmpty(),
                    onSignOut = onSignOut,
                    onRefresh = onRefresh,
                )
            }
        }
    }

    // A sign-in or refresh error floats over the board instead of replacing it.
    state.message?.let { message ->
        Box(
            modifier = Modifier.fillMaxSize().padding(24.dp),
            contentAlignment = Alignment.BottomCenter,
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .background(SurfaceHigh, MaterialTheme.shapes.small)
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = message,
                    style = MaterialTheme.typography.bodySmall,
                    color = TextPrimary,
                    modifier = Modifier.weight(1f),
                )
                TextButton(onClick = onDismissMessage) { Text("Dismiss") }
            }
        }
    }
}

@Composable
private fun BoardTab(
    state: UiState,
    searchVisible: Boolean,
    onToggleSearch: () -> Unit,
    onSearch: (String) -> Unit,
    onClearSearch: () -> Unit,
    onOpen: (String, String) -> Unit,
    onRefresh: () -> Unit,
) {
    if (searchVisible) {
        SearchHeader(
            query = state.searchQuery,
            onQueryChange = onSearch,
            onToggle = onToggleSearch,
            onClear = onClearSearch,
        )
    } else {
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = "Board",
                style = MaterialTheme.typography.headlineMedium,
                color = TextPrimary,
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = onToggleSearch) {
                Icon(Icons.Filled.Search, contentDescription = "Search", tint = TextPrimary)
            }
            IconButton(onClick = onRefresh) {
                Icon(Icons.Filled.Refresh, contentDescription = "Refresh", tint = TextPrimary)
            }
        }
    }

    val searching = state.searchQuery.isNotBlank()
    when {
        searching && state.searchLoading -> LoadingBlock("Searching your addons…")
        searching -> SearchResults(
            results = state.searchResults,
            onOpen = onOpen,
        )
        state.boardLoading && state.board.isEmpty() -> LoadingBlock("Loading your addons…")
        state.boardError != null && state.board.isEmpty() -> MessageBox(
            message = state.boardError ?: "",
            actionLabel = "Try again",
            onAction = onRefresh,
        )
        else -> BoardList(rows = state.board, onOpen = onOpen)
    }
}

@Composable
private fun SearchHeader(
    query: String,
    onQueryChange: (String) -> Unit,
    onToggle: () -> Unit,
    onClear: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        OutlinedTextField(
            value = query,
            onValueChange = onQueryChange,
            singleLine = true,
            placeholder = { Text("Search movies and series") },
            leadingIcon = { Icon(Icons.Filled.Search, contentDescription = null) },
            trailingIcon = {
                if (query.isNotEmpty()) {
                    IconButton(onClick = onClear) {
                        Icon(Icons.Filled.Close, contentDescription = "Clear search")
                    }
                }
            },
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            keyboardActions = KeyboardActions(onSearch = {}),
            modifier = Modifier.weight(1f),
        )
        TextButton(onClick = onToggle) { Text("Close") }
    }
}

@Composable
private fun SearchResults(
    results: List<MetaPreview>,
    onOpen: (String, String) -> Unit,
) {
    if (results.isEmpty()) {
        MessageBox(message = "No addon found a title for this search.")
        return
    }
    LazyVerticalGrid(
        columns = GridCells.Adaptive(minSize = 104.dp),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        gridItems(results, key = { "${it.type}:${it.id}" }) { meta ->
            PosterCard(
                meta = meta,
                onClick = { onOpen(meta.type, meta.id) },
                width = 104.dp,
            )
        }
    }
}

@Composable
private fun BoardList(rows: List<CatalogRow>, onOpen: (String, String) -> Unit) {
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = 24.dp),
    ) {
        items(rows, key = { it.title }) { row ->
            SectionTitle(row.title)
            LazyRow(
                contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                items(row.items, key = { "${it.type}:${it.id}" }) { meta ->
                    PosterCard(
                        meta = meta,
                        onClick = { onOpen(meta.type, meta.id) },
                    )
                }
            }
        }
    }
}

@Composable
private fun AddonsTab(addons: List<AddonDescriptor>, onRefresh: () -> Unit) {
    Column(Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = "Installed addons",
                style = MaterialTheme.typography.headlineMedium,
                color = TextPrimary,
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = onRefresh) {
                Icon(Icons.Filled.Refresh, contentDescription = "Refresh", tint = TextPrimary)
            }
        }
        if (addons.isEmpty()) {
            MessageBox(message = "No addon is installed on this account.")
            return@Column
        }
        LazyColumn(Modifier.fillMaxSize()) {
            items(addons, key = { it.transportUrl }) { addon ->
                Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp)) {
                    Text(
                        text = addon.manifest.name,
                        style = MaterialTheme.typography.titleMedium,
                        color = TextPrimary,
                    )
                    Text(
                        text = addon.manifest.description.orEmpty(),
                        style = MaterialTheme.typography.bodySmall,
                        color = TextMuted,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        text = addon.transportUrl,
                        style = MaterialTheme.typography.bodySmall,
                        color = TextMuted.copy(alpha = 0.7f),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                HorizontalDivider(color = MaterialTheme.colorScheme.outline)
            }
        }
    }
}

@Composable
private fun AccountTab(email: String, onSignOut: () -> Unit, onRefresh: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            text = "Account",
            style = MaterialTheme.typography.headlineMedium,
            color = TextPrimary,
        )
        Text(
            text = email.ifBlank { "Signed in" },
            style = MaterialTheme.typography.bodyMedium,
            color = TextMuted,
        )
        TextButton(onClick = onRefresh) { Text("Reload addons") }
        TextButton(onClick = onSignOut) { Text("Sign out") }
    }
}

@Composable
internal fun MessageBox(
    message: String,
    actionLabel: String? = null,
    onAction: (() -> Unit)? = null,
) {
    MessageBlock(message = message, actionLabel = actionLabel, onAction = onAction)
}
