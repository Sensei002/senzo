package com.senzo.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Settings
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.senzo.app.plugins.PluginProvider
import com.senzo.app.plugins.PluginRepository
import com.senzo.app.plugins.PluginUiState

private val PluginDanger = Color(0xFFFF6B6B)
private val PluginDangerSurface = Color(0x221A0606)

/** Everything the plugins section can ask the view model to do. */
data class PluginActions(
    val addRepository: (String) -> Unit,
    val refreshRepository: (String) -> Unit,
    val removeRepository: (String) -> Unit,
    val setRepositoryEnabled: (String, Boolean) -> Unit,
    val toggleProvider: (String, Boolean) -> Unit,
    val saveSettings: (String, String) -> Unit,
    val dismissError: () -> Unit,
)

/**
 * Plugin repositories and their providers.
 *
 * Repositories publish Nuvio provider scripts; each enabled provider is asked
 * for streams next to the addons. Providers stay off until the viewer turns
 * them on, so installing a repository never starts dozens of network calls.
 */
@Composable
fun PluginsSection(
    modifier: Modifier = Modifier,
    state: PluginUiState,
    actions: PluginActions,
) {
    var showAdd by remember { mutableStateOf(false) }
    var settingsFor by remember { mutableStateOf<PluginProvider?>(null) }
    val config = state.config

    Column(modifier.fillMaxWidth()) {
        state.error?.let { message ->
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 20.dp)
                    .clip(RoundedCornerShape(12.dp))
                    .background(PluginDangerSurface)
                    .padding(horizontal = 14.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = message,
                    style = MaterialTheme.typography.bodySmall,
                    color = PluginDanger,
                    modifier = Modifier.weight(1f),
                )
                TextButton(onClick = actions.dismissError) {
                    Text("Dismiss", color = PluginDanger, style = MaterialTheme.typography.labelMedium)
                }
            }
            Spacer(Modifier.height(12.dp))
        }

        LazyColumn(
            modifier = Modifier.fillMaxWidth(),
            contentPadding = PaddingValues(start = 20.dp, end = 20.dp, bottom = 32.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item {
                Text(
                    text = "Plugin repositories publish provider scripts. The enabled ones look up streams for every title, next to your addons.",
                    style = MaterialTheme.typography.bodySmall,
                    color = TextDim,
                )
            }
            item {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "Repositories",
                        style = MaterialTheme.typography.titleMedium,
                        color = TextPrimary,
                        modifier = Modifier.weight(1f),
                    )
                    Button(
                        onClick = { showAdd = true },
                        shape = RoundedCornerShape(50),
                        colors = ButtonDefaults.buttonColors(containerColor = Accent),
                        contentPadding = PaddingValues(horizontal = 14.dp, vertical = 8.dp),
                    ) {
                        Icon(Icons.Rounded.Add, contentDescription = null, modifier = Modifier.size(16.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Add", style = MaterialTheme.typography.labelMedium)
                    }
                }
            }
            if (config.repositories.isEmpty()) {
                item {
                    EmptyPluginsCard(
                        title = "No repositories yet",
                        body = "Add a plugin manifest URL to install providers such as Torrentio.",
                    )
                }
            }
            items(config.repositories, key = { it.url }) { repository ->
                RepositoryCard(
                    repository = repository,
                    providers = config.providers.filter { it.repository == repository.url },
                    busy = repository.url in state.busy,
                    actions = actions,
                    onSettings = { provider -> settingsFor = provider },
                )
            }
        }
    }

    if (showAdd) {
        AddRepositoryDialog(
            onDismiss = { showAdd = false },
            onAdd = { url ->
                showAdd = false
                actions.addRepository(url)
            },
        )
    }

    settingsFor?.let { provider ->
        SettingsDialog(
            provider = provider,
            onDismiss = { settingsFor = null },
            onSave = { value ->
                settingsFor = null
                actions.saveSettings(provider.key, value)
            },
        )
    }
}

@Composable
private fun RepositoryCard(
    repository: PluginRepository,
    providers: List<PluginProvider>,
    busy: Boolean,
    actions: PluginActions,
    onSettings: (PluginProvider) -> Unit,
) {
    val enabled = providers.count { it.enabled }
    var confirmRemove by remember { mutableStateOf(false) }

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .background(Surface)
            .padding(16.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(
                    text = repository.name.ifBlank { repository.url },
                    style = MaterialTheme.typography.titleSmall,
                    color = TextPrimary,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = repository.url,
                    style = MaterialTheme.typography.bodySmall,
                    color = TextDim,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = when {
                        busy -> "Loading providers…"
                        providers.isEmpty() -> "No providers loaded"
                        else -> "$enabled of ${providers.size} enabled"
                    },
                    style = MaterialTheme.typography.labelSmall,
                    color = TextMuted,
                )
            }
            if (busy) {
                CircularProgressIndicator(
                    modifier = Modifier.size(18.dp),
                    color = Accent,
                    strokeWidth = 2.dp,
                )
            } else {
                IconButton(onClick = { actions.refreshRepository(repository.url) }) {
                    Icon(Icons.Rounded.Refresh, contentDescription = "Refresh", tint = TextMuted)
                }
                IconButton(onClick = { confirmRemove = true }) {
                    Icon(Icons.Rounded.Delete, contentDescription = "Remove", tint = TextDim)
                }
            }
        }

        if (providers.isNotEmpty()) {
            Spacer(Modifier.height(10.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PillButton(text = "Enable all") { actions.setRepositoryEnabled(repository.url, true) }
                PillButton(text = "Disable all") { actions.setRepositoryEnabled(repository.url, false) }
            }
            Spacer(Modifier.height(4.dp))
            providers.forEach { provider ->
                PluginRow(
                    provider = provider,
                    onToggle = { value -> actions.toggleProvider(provider.key, value) },
                    onSettings = { onSettings(provider) },
                )
            }
        }
    }

    if (confirmRemove) {
        AlertDialog(
            onDismissRequest = { confirmRemove = false },
            containerColor = SurfaceHigh,
            title = { Text("Remove repository?", color = TextPrimary) },
            text = {
                Text(
                    text = "Its providers and their settings are removed from this device.",
                    color = TextMuted,
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    confirmRemove = false
                    actions.removeRepository(repository.url)
                }) {
                    Text("Remove", color = PluginDanger)
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmRemove = false }) {
                    Text("Cancel", color = TextMuted)
                }
            },
        )
    }
}

@Composable
private fun PluginRow(
    provider: PluginProvider,
    onToggle: (Boolean) -> Unit,
    onSettings: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(top = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                text = provider.name,
                style = MaterialTheme.typography.bodyMedium,
                color = TextPrimary,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            val subtitle = buildString {
                if (provider.types.isNotEmpty()) append(provider.types.joinToString("/"))
                if (provider.version.isNotBlank()) {
                    if (isNotEmpty()) append(" · ")
                    append("v")
                    append(provider.version)
                }
            }
            if (subtitle.isNotEmpty()) {
                Text(
                    text = subtitle,
                    style = MaterialTheme.typography.labelSmall,
                    color = TextDim,
                )
            }
        }
        if (provider.hasSettings) {
            IconButton(onClick = onSettings, modifier = Modifier.size(34.dp)) {
                Icon(
                    Icons.Rounded.Settings,
                    contentDescription = "Provider settings",
                    tint = TextDim,
                    modifier = Modifier.size(16.dp),
                )
            }
        }
        Switch(
            checked = provider.enabled,
            onCheckedChange = onToggle,
            colors = SwitchDefaults.colors(
                checkedThumbColor = OnAccent,
                checkedTrackColor = Accent,
                uncheckedThumbColor = TextDim,
                uncheckedTrackColor = SurfaceHigh,
            ),
        )
    }
}

@Composable
private fun PillButton(text: String, onClick: () -> Unit) {
    Box(
        modifier = Modifier
            .height(32.dp)
            .clip(RoundedCornerShape(50))
            .background(SurfaceHigh)
            .clickable(onClick = onClick)
            .padding(horizontal = 14.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(text = text, style = MaterialTheme.typography.labelSmall, color = TextMuted)
    }
}

@Composable
private fun EmptyPluginsCard(title: String, body: String) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .background(Surface)
            .padding(16.dp),
    ) {
        Text(title, style = MaterialTheme.typography.titleSmall, color = TextPrimary)
        Spacer(Modifier.height(4.dp))
        Text(body, style = MaterialTheme.typography.bodySmall, color = TextDim)
    }
}

@Composable
private fun AddRepositoryDialog(
    onDismiss: () -> Unit,
    onAdd: (String) -> Unit,
) {
    var url by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = SurfaceHigh,
        title = { Text("Add plugin repository", color = TextPrimary) },
        text = {
            Column {
                Text(
                    text = "Paste the manifest URL of a Nuvio plugin repository. The all-in-one manifest is preinstalled, so this is for repositories you find yourself.",
                    style = MaterialTheme.typography.bodySmall,
                    color = TextMuted,
                )
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(
                    value = url,
                    onValueChange = { url = it },
                    singleLine = true,
                    label = { Text("https://…/manifest.json") },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                )
            }
        },
        confirmButton = {
            TextButton(onClick = { onAdd(url) }, enabled = url.isNotBlank()) {
                Text("Add", color = Accent)
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel", color = TextMuted) }
        },
    )
}

@Composable
private fun SettingsDialog(
    provider: PluginProvider,
    onDismiss: () -> Unit,
    onSave: (String) -> Unit,
) {
    var value by remember { mutableStateOf(provider.settings.ifBlank { "{}" }) }
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = SurfaceHigh,
        title = { Text(provider.name, color = TextPrimary) },
        text = {
            Column {
                Text(
                    text = "Settings are passed to the provider script as JSON.",
                    style = MaterialTheme.typography.bodySmall,
                    color = TextMuted,
                )
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(
                    value = value,
                    onValueChange = { value = it },
                    minLines = 6,
                    maxLines = 10,
                    textStyle = MaterialTheme.typography.bodySmall,
                )
            }
        },
        confirmButton = {
            TextButton(onClick = { onSave(value) }) { Text("Save", color = Accent) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel", color = TextMuted) }
        },
    )
}
