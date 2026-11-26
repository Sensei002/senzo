package com.senzo.app.ui

import android.content.Intent
import android.net.Uri
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Extension
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.KeyboardArrowUp
import androidx.compose.material.icons.rounded.OpenInNew
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Star
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import coil.compose.AsyncImage
import com.senzo.app.AddonDescriptor
import com.senzo.app.CommunityAddon
import com.senzo.app.CommunitySort

/** The two halves of the addon tab: what is installed, and what could be. */
private enum class AddonsSection { Installed, Directory }

/**
 * Addons, split the way the desktop client splits them: the installed ones
 * with their switches and order, and the community directory to install from.
 */
@Composable
fun AddonsScreen(
    addons: List<AddonDescriptor>,
    disabled: Set<String>,
    installing: Boolean,
    community: List<CommunityAddon>,
    communityLoading: Boolean,
    communityError: String?,
    communityQuery: String,
    communitySort: CommunitySort,
    communityHasMore: Boolean,
    communityInstalling: String?,
    onToggle: (String, Boolean) -> Unit,
    onRemove: (String) -> Unit,
    onMove: (String, Boolean) -> Unit,
    onInstall: (String) -> Unit,
    onCommunityQuery: (String) -> Unit,
    onCommunitySort: (CommunitySort) -> Unit,
    onCommunityMore: () -> Unit,
    onCommunityRefresh: () -> Unit,
    onCommunityInstall: (CommunityAddon) -> Unit,
) {
    var section by remember { mutableStateOf(AddonsSection.Installed) }
    var showInstall by remember { mutableStateOf(false) }
    val context = LocalContext.current
    val installed = addons.map { it.transportUrl }.toSet()

    Column(Modifier.fillMaxSize().background(Background).statusBarsPadding()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(start = 20.dp, end = 12.dp, top = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(
                    text = "Addons",
                    style = MaterialTheme.typography.headlineMedium,
                    color = TextPrimary,
                )
                Text(
                    text = if (addons.isEmpty()) {
                        "Nothing installed yet."
                    } else {
                        "${addons.size} installed from your account and this device"
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = TextDim,
                )
            }
            Button(
                onClick = { showInstall = true },
                shape = RoundedCornerShape(50),
                colors = ButtonDefaults.buttonColors(containerColor = Accent),
                contentPadding = PaddingValues(horizontal = 16.dp, vertical = 10.dp),
            ) {
                Icon(Icons.Rounded.Add, contentDescription = null, modifier = Modifier.size(16.dp))
                Spacer(Modifier.width(6.dp))
                Text("Add", style = MaterialTheme.typography.labelMedium)
            }
        }
        Spacer(Modifier.height(12.dp))
        SectionSwitch(section = section, onSelect = { section = it })
        Spacer(Modifier.height(12.dp))
        when (section) {
            AddonsSection.Installed -> InstalledSection(
                modifier = Modifier.weight(1f),
                addons = addons,
                disabled = disabled,
                onToggle = onToggle,
                onRemove = onRemove,
                onMove = onMove,
                onAdd = { showInstall = true },
            )

            AddonsSection.Directory -> DirectorySection(
                modifier = Modifier.weight(1f),
                addons = community,
                installed = installed,
                loading = communityLoading,
                error = communityError,
                query = communityQuery,
                sort = communitySort,
                hasMore = communityHasMore,
                installingUrl = communityInstalling,
                onQuery = onCommunityQuery,
                onSort = onCommunitySort,
                onMore = onCommunityMore,
                onRefresh = onCommunityRefresh,
                onInstall = onCommunityInstall,
                onOpenDirectory = {
                    runCatching {
                        context.startActivity(
                            Intent(Intent.ACTION_VIEW, Uri.parse("https://stremio-addons.net")),
                        )
                    }
                },
            )
        }
    }

    if (showInstall) {
        var url by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { showInstall = false },
            containerColor = Surface,
            title = { Text("Install addon", color = TextPrimary) },
            text = {
                Column {
                    Text(
                        text = "Paste the URL of an addon. The manifest is read first, so a wrong address is rejected here.",
                        style = MaterialTheme.typography.bodySmall,
                        color = TextMuted,
                    )
                    Spacer(Modifier.height(12.dp))
                    OutlinedTextField(
                        value = url,
                        onValueChange = { url = it },
                        placeholder = { Text("https://example.com/addon", color = TextDim) },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        onInstall(url)
                        url = ""
                        showInstall = false
                    },
                    enabled = url.isNotBlank() && !installing,
                ) {
                    Text(if (installing) "Installing…" else "Install", color = Accent)
                }
            },
            dismissButton = {
                TextButton(onClick = { showInstall = false }) {
                    Text("Cancel", color = TextMuted)
                }
            },
        )
    }
}

@Composable
private fun SectionSwitch(section: AddonsSection, onSelect: (AddonsSection) -> Unit) {
    Row(
        modifier = Modifier
            .padding(horizontal = 16.dp)
            .fillMaxWidth()
            .clip(RoundedCornerShape(14.dp))
            .background(Surface)
            .padding(4.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Pill(
            label = "Installed",
            selected = section == AddonsSection.Installed,
            modifier = Modifier.weight(1f),
            onClick = { onSelect(AddonsSection.Installed) },
        )
        Pill(
            label = "Directory",
            selected = section == AddonsSection.Directory,
            modifier = Modifier.weight(1f),
            onClick = { onSelect(AddonsSection.Directory) },
        )
    }
}

@Composable
private fun Pill(
    label: String,
    selected: Boolean,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    Box(
        modifier = modifier
            .clip(RoundedCornerShape(11.dp))
            .background(if (selected) Accent.copy(alpha = 0.16f) else Color.Transparent)
            .clickable(onClick = onClick)
            .padding(vertical = 9.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = if (selected) Accent else TextMuted,
        )
    }
}

@Composable
private fun InstalledSection(
    addons: List<AddonDescriptor>,
    disabled: Set<String>,
    onToggle: (String, Boolean) -> Unit,
    onRemove: (String) -> Unit,
    onMove: (String, Boolean) -> Unit,
    onAdd: () -> Unit,
    modifier: Modifier = Modifier,
) {
    if (addons.isEmpty()) {
        EmptyState(
            modifier = modifier,
            title = "No addons installed",
            body = "Addons bring catalogues, metadata and streams. Install one from the directory or with its URL.",
            icon = Icons.Rounded.Extension,
            action = "Browse the directory",
            onAction = onAdd,
        )
        return
    }
    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        items(addons, key = { it.transportUrl }) { addon ->
            AddonCard(
                addon = addon,
                enabled = addon.transportUrl !in disabled,
                onToggle = { onToggle(addon.transportUrl, it) },
                onRemove = { onRemove(addon.transportUrl) },
                onMove = { up -> onMove(addon.transportUrl, up) },
            )
        }
    }
}

@Composable
private fun DirectorySection(
    addons: List<CommunityAddon>,
    installed: Set<String>,
    loading: Boolean,
    error: String?,
    query: String,
    sort: CommunitySort,
    hasMore: Boolean,
    installingUrl: String?,
    onQuery: (String) -> Unit,
    onSort: (CommunitySort) -> Unit,
    onMore: () -> Unit,
    onRefresh: () -> Unit,
    onInstall: (CommunityAddon) -> Unit,
    onOpenDirectory: () -> Unit,
    modifier: Modifier = Modifier,
) {
    LaunchedEffect(Unit) { if (addons.isEmpty() && !loading) onRefresh() }
    Column(modifier = modifier.fillMaxWidth()) {
        OutlinedTextField(
            value = query,
            onValueChange = onQuery,
            placeholder = { Text("Search the directory", color = TextDim) },
            leadingIcon = {
                Icon(Icons.Rounded.Search, contentDescription = null, tint = TextDim)
            },
            singleLine = true,
            shape = RoundedCornerShape(50),
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp),
        )
        Spacer(Modifier.height(10.dp))
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            CommunitySort.entries.forEach { option ->
                Box(
                    modifier = Modifier
                        .clip(RoundedCornerShape(50))
                        .background(
                            if (option == sort) Accent.copy(alpha = 0.16f) else Surface,
                        )
                        .clickable { onSort(option) }
                        .padding(horizontal = 14.dp, vertical = 8.dp),
                ) {
                    Text(
                        text = option.label,
                        style = MaterialTheme.typography.labelMedium,
                        color = if (option == sort) Accent else TextMuted,
                    )
                }
            }
        }
        Spacer(Modifier.height(10.dp))
        when {
            error != null && addons.isEmpty() -> {
                ErrorBlock(message = error, onRetry = onRefresh)
            }

            loading && addons.isEmpty() -> {
                LoadingBlock("Reading the directory…")
            }

            addons.isEmpty() -> {
                EmptyState(
                    title = "Nothing matches",
                    body = "No addon in the directory matches that search.",
                    icon = Icons.Rounded.Search,
                )
            }

            else -> {
                LazyColumn(
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    items(addons, key = { it.manifestUrl }) { addon ->
                        CommunityCard(
                            addon = addon,
                            installed = addon.baseUrl in installed,
                            installing = installingUrl == addon.manifestUrl,
                            onInstall = { onInstall(addon) },
                        )
                    }
                    item(key = "footer") {
                        Column(
                            modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                        ) {
                            if (loading) {
                                CircularProgressIndicator(
                                    color = Accent,
                                    strokeWidth = 2.dp,
                                    modifier = Modifier.size(20.dp),
                                )
                                Spacer(Modifier.height(8.dp))
                            }
                            if (hasMore && !loading) {
                                TextButton(onClick = onMore) {
                                    Text("Load more", color = Accent)
                                }
                            }
                            TextButton(onClick = onOpenDirectory) {
                                Icon(
                                    Icons.Rounded.OpenInNew,
                                    contentDescription = null,
                                    tint = TextDim,
                                    modifier = Modifier.size(14.dp),
                                )
                                Spacer(Modifier.width(6.dp))
                                Text(
                                    text = "Browse stremio-addons.net",
                                    style = MaterialTheme.typography.labelMedium,
                                    color = TextDim,
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun CommunityCard(
    addon: CommunityAddon,
    installed: Boolean,
    installing: Boolean,
    onInstall: () -> Unit,
) {
    val context = LocalContext.current
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .background(Surface)
            .padding(12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(
                modifier = Modifier
                    .size(44.dp)
                    .clip(RoundedCornerShape(12.dp))
                    .background(SurfaceHigh),
                contentAlignment = Alignment.Center,
            ) {
                AsyncImage(
                    model = addon.logo,
                    contentDescription = addon.name,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.fillMaxSize(),
                )
            }
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(
                    text = addon.name,
                    style = MaterialTheme.typography.titleSmall,
                    color = TextPrimary,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(
                        Icons.Rounded.Star,
                        contentDescription = null,
                        tint = TextDim,
                        modifier = Modifier.size(12.dp),
                    )
                    Spacer(Modifier.width(3.dp))
                    Text(
                        text = stars(addon.stars),
                        style = MaterialTheme.typography.labelSmall,
                        color = TextDim,
                    )
                    Text(
                        text = " · ${addon.kind}",
                        style = MaterialTheme.typography.labelSmall,
                        color = TextDim,
                    )
                }
            }
            Spacer(Modifier.width(8.dp))
            when {
                installed -> Text(
                    text = "Installed",
                    style = MaterialTheme.typography.labelMedium,
                    color = Accent,
                )

                installing -> CircularProgressIndicator(
                    color = Accent,
                    strokeWidth = 2.dp,
                    modifier = Modifier.size(18.dp),
                )

                else -> Button(
                    onClick = onInstall,
                    shape = RoundedCornerShape(50),
                    colors = ButtonDefaults.buttonColors(
                        containerColor = Accent.copy(alpha = 0.16f),
                        contentColor = Accent,
                    ),
                    contentPadding = PaddingValues(horizontal = 14.dp, vertical = 8.dp),
                ) {
                    Text("Install", style = MaterialTheme.typography.labelMedium)
                }
            }
        }
        if (addon.description.isNotBlank()) {
            Spacer(Modifier.height(8.dp))
            Text(
                text = addon.description,
                style = MaterialTheme.typography.bodySmall,
                color = TextMuted,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (addon.categories.isNotEmpty() || addon.p2p) {
            Spacer(Modifier.height(8.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                if (addon.p2p) {
                    LabelPill("P2P", color = Accent.copy(alpha = 0.16f))
                }
                addon.categories.take(3).forEach { category ->
                    LabelPill(category, color = SurfaceHigh)
                }
            }
        }
        if (addon.configurable && addon.configurationUrl != null && !installed) {
            TextButton(
                onClick = {
                    runCatching {
                        context.startActivity(
                            Intent(Intent.ACTION_VIEW, Uri.parse(addon.configurationUrl)),
                        )
                    }
                },
                contentPadding = PaddingValues(0.dp),
            ) {
                Icon(
                    Icons.Rounded.OpenInNew,
                    contentDescription = null,
                    tint = Accent,
                    modifier = Modifier.size(14.dp),
                )
                Spacer(Modifier.width(6.dp))
                Text(
                    text = "Configure before installing",
                    style = MaterialTheme.typography.labelSmall,
                    color = Accent,
                )
            }
        }
    }
}

/** Stars as a short number, which is all a directory row needs. */
private fun stars(value: Long): String = when {
    value >= 1_000_000 -> "${value / 1_000_000}.${(value % 1_000_000) / 100_000}M"
    value >= 1_000 -> "${value / 1_000}.${(value % 1_000) / 100}k"
    else -> value.toString()
}

@Composable
private fun AddonCard(
    addon: AddonDescriptor,
    enabled: Boolean,
    onToggle: (Boolean) -> Unit,
    onRemove: () -> Unit,
    onMove: (Boolean) -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .background(Surface)
            .padding(12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(
                modifier = Modifier
                    .size(44.dp)
                    .clip(RoundedCornerShape(12.dp))
                    .background(SurfaceHigh),
                contentAlignment = Alignment.Center,
            ) {
                AsyncImage(
                    model = addon.manifest.logo,
                    contentDescription = addon.manifest.name,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.fillMaxSize(),
                )
            }
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(
                    text = addon.manifest.name,
                    style = MaterialTheme.typography.titleSmall,
                    color = TextPrimary,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = listOfNotNull(
                        addon.manifest.version?.let { "v$it" },
                        addon.manifest.types.joinToString(", ").takeIf { it.isNotBlank() },
                    ).joinToString(" · "),
                    style = MaterialTheme.typography.bodySmall,
                    color = TextDim,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Switch(
                checked = enabled,
                onCheckedChange = onToggle,
                colors = SwitchDefaults.colors(
                    checkedThumbColor = Color.White,
                    checkedTrackColor = Accent,
                ),
            )
        }
        addon.manifest.description?.takeIf { it.isNotBlank() }?.let { description ->
            Spacer(Modifier.height(8.dp))
            Text(
                text = description,
                style = MaterialTheme.typography.bodySmall,
                color = TextMuted,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Row(
            modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = if (enabled) "Used on the board" else "Hidden from the board",
                style = MaterialTheme.typography.labelSmall,
                color = if (enabled) Accent else TextDim,
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = { onMove(true) }, modifier = Modifier.size(32.dp)) {
                Icon(
                    Icons.Rounded.KeyboardArrowUp,
                    contentDescription = "Move up",
                    tint = TextMuted,
                    modifier = Modifier.size(18.dp),
                )
            }
            IconButton(onClick = { onMove(false) }, modifier = Modifier.size(32.dp)) {
                Icon(
                    Icons.Rounded.KeyboardArrowDown,
                    contentDescription = "Move down",
                    tint = TextMuted,
                    modifier = Modifier.size(18.dp),
                )
            }
            IconButton(onClick = onRemove, modifier = Modifier.size(32.dp)) {
                Icon(
                    Icons.Rounded.Delete,
                    contentDescription = "Remove addon",
                    tint = Color(0xFFFF6B6B),
                    modifier = Modifier.size(18.dp),
                )
            }
        }
    }
}
