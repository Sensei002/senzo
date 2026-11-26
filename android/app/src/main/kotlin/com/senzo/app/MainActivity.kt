package com.senzo.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.Alignment
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Bookmark
import androidx.compose.material.icons.rounded.Extension
import androidx.compose.material.icons.rounded.Home
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.senzo.app.ui.Accent
import com.senzo.app.ui.AddonsScreen
import com.senzo.app.ui.Background
import com.senzo.app.ui.BackgroundElevated
import com.senzo.app.ui.DetailsScreen
import com.senzo.app.ui.HomeScreen
import com.senzo.app.ui.LibraryScreen
import com.senzo.app.ui.LoginScreen
import com.senzo.app.ui.Outline
import com.senzo.app.ui.PlayerScreen
import com.senzo.app.ui.SearchScreen
import com.senzo.app.ui.SenzoTheme
import com.senzo.app.ui.SettingsScreen
import com.senzo.app.ui.TextDim

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        CrashReporter.install(this)

        // A crash the last run could not survive becomes a page that can be
        // read and copied, instead of a second silent failure.
        val previous = CrashReporter.pending(this)
        if (previous != null) {
            showCrash(previous)
            return
        }

        try {
            setContent {
                SenzoTheme {
                    SenzoApp()
                }
            }
        } catch (error: Throwable) {
            showCrash(CrashReporter.record(this, error))
        }
    }

    private fun showCrash(details: String) {
        setContentView(
            CrashReporter.page(this, details) {
                CrashReporter.forget(this)
                recreate()
            },
        )
    }
}

@Composable
private fun SenzoApp(viewModel: MainViewModel = viewModel()) {
    val state by viewModel.state.collectAsStateWithLifecycle()

    val playback = state.playback
    if (playback != null) {
        BackHandler { viewModel.stopPlayback() }
        PlayerScreen(
            playback = playback,
            subtitleSize = state.settings.subtitleSize,
            autoNext = state.settings.autoplayNext,
            onProgress = viewModel::reportProgress,
            onNext = viewModel::playNext,
            onClose = { position, duration -> viewModel.stopPlayback(position, duration) },
        )
        return
    }

    when (state.screen) {
        Screen.Login -> LoginScreen(
            signingIn = state.signingIn,
            message = state.message,
            onSignIn = viewModel::signIn,
            onGuest = viewModel::continueAsGuest,
        )

        is Screen.Details -> {
            BackHandler { viewModel.back() }
            val details = state.details
            if (details == null) {
                LaunchedEffect(Unit) { viewModel.back() }
            } else {
                DetailsScreen(
                    state = details,
                    onBack = viewModel::back,
                    onPlayBest = viewModel::playBest,
                    onPlayStream = viewModel::play,
                    onSelectSeason = viewModel::selectSeason,
                    onSelectVideo = viewModel::selectVideo,
                    onToggleLibrary = viewModel::toggleLibrary,
                    onRetry = viewModel::reloadStreams,
                )
            }
        }

        Screen.Main -> MainShell(state = state, viewModel = viewModel)
    }
}

/** The five tabs the app lives in once signed in. */
@Composable
private fun MainShell(state: UiState, viewModel: MainViewModel) {
    val snackbar = remember { SnackbarHostState() }
    val message = state.message

    LaunchedEffect(message) {
        if (message != null) {
            snackbar.showSnackbar(message)
            viewModel.dismissMessage()
        }
    }

    BackHandler(enabled = state.tab != Tab.Home) { viewModel.selectTab(Tab.Home) }

    Scaffold(
        containerColor = Background,
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
        snackbarHost = { SnackbarHost(snackbar) },
        bottomBar = {
            BottomBar(tab = state.tab, onSelect = viewModel::selectTab)
        },
    ) { innerPadding ->
        Box(Modifier.fillMaxSize().padding(innerPadding)) {
            when (state.tab) {
                Tab.Home -> HomeScreen(
                    state = state,
                    onOpen = { preview -> viewModel.openDetails(preview.type, preview.id, preview) },
                    onPlay = viewModel::playFromBoard,
                    onResume = { entry -> viewModel.openDetails(entry.type, entry.metaId) },
                    onRefresh = viewModel::refresh,
                    onSearch = { viewModel.selectTab(Tab.Search) },
                    onProfile = { viewModel.selectTab(Tab.Settings) },
                    onForgetProgress = viewModel::forgetProgress,
                )

                Tab.Search -> SearchScreen(
                    query = state.searchQuery,
                    results = state.searchResults,
                    loading = state.searchLoading,
                    recent = state.recentSearches,
                    onQuery = viewModel::search,
                    onSubmit = viewModel::submitSearch,
                    onClearQuery = viewModel::clearSearch,
                    onUseRecent = { value ->
                        viewModel.search(value)
                        viewModel.submitSearch()
                    },
                    onClearRecent = viewModel::clearRecentSearches,
                    onOpen = { preview -> viewModel.openDetails(preview.type, preview.id, preview) },
                )

                Tab.Library -> LibraryScreen(
                    entries = state.library,
                    onOpen = { preview -> viewModel.openDetails(preview.type, preview.id, preview) },
                    onRemove = viewModel::removeFromLibrary,
                    onBrowse = { viewModel.selectTab(Tab.Home) },
                )

                Tab.Addons -> AddonsScreen(
                    addons = state.addons,
                    disabled = state.disabledAddons,
                    installing = state.installing,
                    onToggle = viewModel::toggleAddon,
                    onRemove = viewModel::removeAddon,
                    onMove = viewModel::moveAddon,
                    onInstall = viewModel::installAddon,
                )

                Tab.Settings -> SettingsScreen(
                    session = state.session,
                    settings = state.settings,
                    onAutoplay = { value ->
                        viewModel.updateSettings { it.copy(autoplayNext = value) }
                    },
                    onResume = { value ->
                        viewModel.updateSettings { it.copy(resumePlayback = value) }
                    },
                    onQuality = { value ->
                        viewModel.updateSettings { it.copy(preferHighestQuality = value) }
                    },
                    onSubtitleSize = { value ->
                        viewModel.updateSettings { it.copy(subtitleSize = value) }
                    },
                    onClearHistory = viewModel::clearWatchHistory,
                    onClearSearches = viewModel::clearRecentSearches,
                    onSignOut = viewModel::signOut,
                )
            }
        }
    }
}

private data class TabSpec(val tab: Tab, val label: String, val icon: ImageVector)

private val tabs = listOf(
    TabSpec(Tab.Home, "Home", Icons.Rounded.Home),
    TabSpec(Tab.Search, "Search", Icons.Rounded.Search),
    TabSpec(Tab.Library, "Library", Icons.Rounded.Bookmark),
    TabSpec(Tab.Addons, "Addons", Icons.Rounded.Extension),
    TabSpec(Tab.Settings, "Settings", Icons.Rounded.Settings),
)

/**
 * The tab bar, drawn rather than borrowed from Material.
 *
 * The stock NavigationBar brings a grey container and a full-width indicator,
 * which sits badly under a poster board. This keeps the surface almost the
 * colour of the page, marks the current tab with a cyan capsule around the
 * icon, and fades into the artwork with a hairline on top.
 */
@Composable
private fun BottomBar(tab: Tab, onSelect: (Tab) -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(Background.copy(alpha = 0.97f)),
    ) {
        Box(
            Modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(
                    Brush.horizontalGradient(
                        listOf(Color.Transparent, Outline.copy(alpha = 0.9f), Color.Transparent),
                    ),
                ),
        )
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .padding(horizontal = 6.dp, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            tabs.forEach { spec ->
                val selected = spec.tab == tab
                Column(
                    modifier = Modifier
                        .weight(1f)
                        .clip(RoundedCornerShape(18.dp))
                        .clickable { onSelect(spec.tab) }
                        .padding(vertical = 4.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Box(
                        modifier = Modifier
                            .clip(RoundedCornerShape(12.dp))
                            .background(
                                if (selected) Accent.copy(alpha = 0.14f) else Color.Transparent,
                            )
                            .padding(horizontal = 16.dp, vertical = 4.dp),
                    ) {
                        Icon(
                            imageVector = spec.icon,
                            contentDescription = spec.label,
                            tint = if (selected) Accent else TextDim,
                            modifier = Modifier.size(21.dp),
                        )
                    }
                    Spacer(Modifier.height(3.dp))
                    Text(
                        text = spec.label,
                        style = MaterialTheme.typography.labelSmall,
                        color = if (selected) Accent else TextDim,
                    )
                }
            }
        }
    }
}