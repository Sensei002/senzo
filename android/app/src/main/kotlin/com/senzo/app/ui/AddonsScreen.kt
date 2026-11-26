package com.senzo.app.ui

import androidx.compose.foundation.background
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
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import coil.compose.AsyncImage
import com.senzo.app.AddonDescriptor

/**
 * Installed addons: what they serve, whether they are used, their order and
 * the address the viewer typed to install one.
 */
@Composable
fun AddonsScreen(
    addons: List<AddonDescriptor>,
    disabled: Set<String>,
    installing: Boolean,
    onToggle: (String, Boolean) -> Unit,
    onRemove: (String) -> Unit,
    onMove: (String, Boolean) -> Unit,
    onInstall: (String) -> Unit,
) {
    var showInstall by remember { mutableStateOf(false) }
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
        Spacer(Modifier.height(10.dp))
        if (addons.isEmpty()) {
            EmptyState(
                title = "No addons installed",
                body = "Addons bring catalogues, metadata and streams. Install one with its URL.",
                icon = Icons.Rounded.Extension,
                action = "Install an addon",
                onAction = { showInstall = true },
            )
        } else {
            LazyColumn(
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