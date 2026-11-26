package com.senzo.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Bookmark
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import com.senzo.app.LibraryEntry
import com.senzo.app.MetaPreview

/** The titles the viewer saved, newest first. */
@Composable
fun LibraryScreen(
    entries: List<LibraryEntry>,
    onOpen: (MetaPreview) -> Unit,
    onRemove: (String) -> Unit,
    onBrowse: () -> Unit,
) {
    Column(Modifier.fillMaxSize().background(Background).statusBarsPadding()) {
        Column(Modifier.padding(start = 20.dp, top = 10.dp, end = 20.dp, bottom = 4.dp)) {
            Text(
                text = "My Library",
                style = MaterialTheme.typography.headlineMedium,
                color = TextPrimary,
            )
            Text(
                text = if (entries.isEmpty()) {
                    "Saved titles stay on this device."
                } else {
                    "${entries.size} saved " + if (entries.size == 1) "title" else "titles"
                },
                style = MaterialTheme.typography.bodySmall,
                color = TextDim,
            )
        }
        if (entries.isEmpty()) {
            EmptyState(
                title = "Your library is empty",
                body = "Open a title and tap Save to keep it here.",
                icon = Icons.Rounded.Bookmark,
                action = "Browse the board",
                onAction = onBrowse,
            )
        } else {
            LazyVerticalGrid(
                columns = GridCells.Fixed(3),
                contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 24.dp),
                verticalArrangement = Arrangement.spacedBy(14.dp),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                items(entries, key = { it.item.key }) { entry ->
                    Box {
                        PosterCard(
                            item = entry.item,
                            modifier = Modifier.fillMaxWidth(),
                            onClick = { onOpen(entry.item) },
                        )
                        Box(
                            modifier = Modifier
                                .align(Alignment.TopStart)
                                .padding(6.dp)
                                .size(22.dp)
                                .clip(CircleShape)
                                .background(Color.Black.copy(alpha = 0.6f))
                                .clickable { onRemove(entry.item.key) },
                            contentAlignment = Alignment.Center,
                        ) {
                            Icon(
                                imageVector = Icons.Rounded.Close,
                                contentDescription = "Remove from library",
                                tint = Color.White,
                                modifier = Modifier.size(14.dp),
                            )
                        }
                    }
                }
            }
        }
    }
}