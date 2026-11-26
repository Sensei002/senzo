package com.senzo.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.History
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import com.senzo.app.MetaPreview

/** Search across every installed addon that supports the `search` extra. */
@Composable
fun SearchScreen(
    query: String,
    results: List<MetaPreview>,
    loading: Boolean,
    recent: List<String>,
    onQuery: (String) -> Unit,
    onSubmit: () -> Unit,
    onClearQuery: () -> Unit,
    onUseRecent: (String) -> Unit,
    onClearRecent: () -> Unit,
    onOpen: (MetaPreview) -> Unit,
) {
    Column(Modifier.fillMaxSize().background(Background).statusBarsPadding()) {
        Text(
            text = "Search",
            style = MaterialTheme.typography.headlineMedium,
            color = TextPrimary,
            modifier = Modifier.padding(start = 20.dp, top = 10.dp, bottom = 12.dp),
        )
        TextField(
            value = query,
            onValueChange = onQuery,
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .clip(RoundedCornerShape(16.dp)),
            placeholder = {
                Text("Movies, series and more", color = TextDim)
            },
            leadingIcon = {
                Icon(Icons.Rounded.Search, contentDescription = null, tint = TextMuted)
            },
            trailingIcon = {
                if (query.isNotEmpty()) {
                    IconButton(onClick = onClearQuery) {
                        Icon(Icons.Rounded.Close, contentDescription = "Clear search", tint = TextMuted)
                    }
                }
            },
            singleLine = true,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            keyboardActions = KeyboardActions(onSearch = { onSubmit() }),
            colors = TextFieldDefaults.colors(
                focusedContainerColor = Surface,
                unfocusedContainerColor = Surface,
                focusedIndicatorColor = Color.Transparent,
                unfocusedIndicatorColor = Color.Transparent,
                cursorColor = Accent,
                focusedTextColor = TextPrimary,
                unfocusedTextColor = TextPrimary,
            ),
        )
        if (query.isEmpty() && recent.isNotEmpty()) {
            Row(
                modifier = Modifier.fillMaxWidth().padding(start = 20.dp, end = 12.dp, top = 20.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = "Recent",
                    style = MaterialTheme.typography.titleSmall,
                    color = TextPrimary,
                    modifier = Modifier.weight(1f),
                )
                Text(
                    text = "Clear",
                    style = MaterialTheme.typography.labelMedium,
                    color = TextMuted,
                    modifier = Modifier
                        .clip(RoundedCornerShape(50))
                        .clickable(onClick = onClearRecent)
                        .padding(horizontal = 10.dp, vertical = 6.dp),
                )
            }
            LazyRow(
                contentPadding = PaddingValues(horizontal = 20.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                items(recent, key = { it }) { term ->
                    Row(
                        modifier = Modifier
                            .clip(RoundedCornerShape(50))
                            .background(Surface)
                            .clickable { onUseRecent(term) }
                            .padding(horizontal = 12.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Icon(
                            Icons.Rounded.History,
                            contentDescription = null,
                            tint = TextDim,
                            modifier = Modifier.size(14.dp),
                        )
                        Text(term, style = MaterialTheme.typography.bodySmall, color = TextMuted)
                    }
                }
            }
        }
        Spacer(Modifier.height(12.dp))
        when {
            loading -> LoadingBlock("Searching your addons…")
            query.isBlank() -> EmptyState(
                title = "Find something to watch",
                body = "Search runs against every addon that supports it, so one query reaches your whole library.",
                icon = Icons.Rounded.Search,
            )
            results.isEmpty() -> EmptyState(
                title = "No matches",
                body = "Nothing matched \"$query\". Try a shorter title.",
                icon = Icons.Rounded.Search,
            )
            else -> LazyVerticalGrid(
                columns = GridCells.Fixed(3),
                contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp),
                verticalArrangement = Arrangement.spacedBy(14.dp),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                items(results, key = { it.key }) { item ->
                    PosterCard(item = item, onClick = { onOpen(item) })
                }
            }
        }
    }
}