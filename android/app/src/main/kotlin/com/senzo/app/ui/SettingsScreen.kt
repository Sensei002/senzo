package com.senzo.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Logout
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.ui.unit.dp
import com.senzo.app.AppSettings
import com.senzo.app.BuildConfig
import com.senzo.app.Session

/** Account, playback preferences, saved data and the build information. */
@Composable
fun SettingsScreen(
    session: Session?,
    settings: AppSettings,
    onAutoplay: (Boolean) -> Unit,
    onResume: (Boolean) -> Unit,
    onQuality: (Boolean) -> Unit,
    onEngine: (String) -> Unit,
    onSubtitleSize: (Int) -> Unit,
    onClearHistory: () -> Unit,
    onClearSearches: () -> Unit,
    onSignOut: () -> Unit,
) {
    var confirmSignOut by remember { mutableStateOf(false) }
    // A guest profile is local to this device; the account card says so
    // instead of claiming an account that is not there.
    val accountDetail = if (session == null) {
        "Local profile: the official addons, no account"
    } else {
        "The same account the desktop app uses"
    }
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(Background)
            .statusBarsPadding()
            .verticalScroll(rememberScrollState())
            .padding(bottom = 28.dp),
    ) {
        Text(
            text = "Settings",
            style = MaterialTheme.typography.headlineMedium,
            color = TextPrimary,
            modifier = Modifier.padding(start = 20.dp, end = 20.dp, top = 10.dp, bottom = 12.dp),
        )

        SectionTitle("Account")
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .clip(RoundedCornerShape(16.dp))
                .background(Surface)
                .padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(
                    text = session?.email ?: "Guest",
                    style = MaterialTheme.typography.titleSmall,
                    color = TextPrimary,
                )
                Text(
                    text = accountDetail,
                    style = MaterialTheme.typography.bodySmall,
                    color = TextDim,
                )
            }
            if (session == null) {
                Row(
                    modifier = Modifier
                        .clip(RoundedCornerShape(50))
                        .background(Accent.copy(alpha = 0.16f))
                        .clickable { onSignOut() }
                        .padding(horizontal = 14.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "Sign in",
                        style = MaterialTheme.typography.labelMedium,
                        color = Accent,
                    )
                }
            } else {
                Row(
                    modifier = Modifier
                        .clip(RoundedCornerShape(50))
                        .background(Color(0xFFFF6B6B).copy(alpha = 0.16f))
                        .clickable { confirmSignOut = true }
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(
                        Icons.AutoMirrored.Rounded.Logout,
                        contentDescription = null,
                        tint = Color(0xFFFF6B6B),
                        modifier = Modifier.size(15.dp),
                    )
                    Spacer(Modifier.width(6.dp))
                    Text(
                        text = "Sign out",
                        style = MaterialTheme.typography.labelMedium,
                        color = Color(0xFFFF6B6B),
                    )
                }
            }
        }

        SectionTitle("Playback")
        SettingSwitch(
            title = "Autoplay next episode",
            subtitle = "Start the next episode when one finishes.",
            checked = settings.autoplayNext,
            onChange = onAutoplay,
        )
        SettingSwitch(
            title = "Resume where you left off",
            subtitle = "Continue from your last position on this device.",
            checked = settings.resumePlayback,
            onChange = onResume,
        )
        SettingSwitch(
            title = "Prefer the best quality",
            subtitle = "Sort a title's streams by resolution and seeders.",
            checked = settings.preferHighestQuality,
            onChange = onQuality,
        )

        Text(
            text = "Player engine",
            style = MaterialTheme.typography.labelMedium,
            color = TextMuted,
            modifier = Modifier.padding(horizontal = 16.dp).padding(top = 8.dp),
        )
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            listOf("media3" to "Media3", "libmpv" to "libmpv").forEach { (value, label) ->
                val selected = settings.playerEngine == value
                Box(
                    modifier = Modifier
                        .weight(1f)
                        .height(40.dp)
                        .clip(RoundedCornerShape(12.dp))
                        .background(if (selected) Accent else Surface)
                        .clickable { onEngine(value) },
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = label,
                        style = MaterialTheme.typography.labelMedium,
                        color = if (selected) OnAccent else TextMuted,
                    )
                }
            }
        }
        Text(
            text = "Media3 uses the phone's decoders; libmpv is the desktop player and decodes everything itself.",
            style = MaterialTheme.typography.bodySmall,
            color = TextDim,
            modifier = Modifier.padding(horizontal = 16.dp).padding(bottom = 8.dp),
        )

        SectionTitle("Subtitles")
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            listOf(0 to "Small", 1 to "Medium", 2 to "Large").forEach { (value, label) ->
                val selected = settings.subtitleSize == value
                Box(
                    modifier = Modifier
                        .weight(1f)
                        .height(40.dp)
                        .clip(RoundedCornerShape(12.dp))
                        .background(if (selected) Accent else Surface)
                        .clickable { onSubtitleSize(value) },
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = label,
                        style = MaterialTheme.typography.labelMedium,
                        color = if (selected) OnAccent else TextMuted,
                    )
                }
            }
        }

        SectionTitle("On this device")
        ActionRow(
            title = "Clear watch history",
            subtitle = "Removes the resume points of the board.",
            onClick = onClearHistory,
        )
        ActionRow(
            title = "Clear recent searches",
            subtitle = "Removes the suggestions under the search field.",
            onClick = onClearSearches,
        )

        SectionTitle("About")
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .clip(RoundedCornerShape(16.dp))
                .background(Surface)
                .padding(16.dp),
        ) {
            Text(
                text = "SENZO for Android",
                style = MaterialTheme.typography.titleSmall,
                color = TextPrimary,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                text = "Version ${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})",
                style = MaterialTheme.typography.bodySmall,
                color = TextDim,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                text = "The mobile client of the same addon-driven library the desktop app shows. No account data leaves the addons you installed.",
                style = MaterialTheme.typography.bodySmall,
                color = TextMuted,
            )
        }
    }

    if (confirmSignOut) {
        AlertDialog(
            onDismissRequest = { confirmSignOut = false },
            containerColor = Surface,
            title = { Text("Sign out?", color = TextPrimary) },
            text = {
                Text(
                    text = "The library and watch history on this device stay, but the board needs a sign-in again.",
                    style = MaterialTheme.typography.bodySmall,
                    color = TextMuted,
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmSignOut = false
                        onSignOut()
                    },
                ) { Text("Sign out", color = Color(0xFFFF6B6B)) }
            },
            dismissButton = {
                TextButton(onClick = { confirmSignOut = false }) {
                    Text("Cancel", color = TextMuted)
                }
            },
        )
    }
}

@Composable
private fun SectionTitle(title: String) {
    Text(
        text = title.uppercase(),
        style = MaterialTheme.typography.labelSmall,
        color = TextDim,
        modifier = Modifier.padding(start = 20.dp, top = 22.dp, bottom = 8.dp),
    )
}

@Composable
private fun SettingSwitch(
    title: String,
    subtitle: String,
    checked: Boolean,
    onChange: (Boolean) -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 6.dp)
            .clip(RoundedCornerShape(16.dp))
            .background(Surface)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleSmall, color = TextPrimary)
            Text(subtitle, style = MaterialTheme.typography.bodySmall, color = TextDim)
        }
        Spacer(Modifier.width(12.dp))
        Switch(
            checked = checked,
            onCheckedChange = onChange,
            colors = SwitchDefaults.colors(
                checkedThumbColor = Color.White,
                checkedTrackColor = Accent,
            ),
        )
    }
}

@Composable
private fun ActionRow(title: String, subtitle: String, onClick: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 6.dp)
            .clip(RoundedCornerShape(16.dp))
            .background(Surface)
            .clickable(onClick = onClick)
            .padding(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleSmall, color = TextPrimary)
            Text(subtitle, style = MaterialTheme.typography.bodySmall, color = TextDim)
        }
    }
}