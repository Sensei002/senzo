package com.senzo.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
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
import com.senzo.app.ui.PlayerScreen
import com.senzo.app.ui.SearchScreen
import com.senzo.app.ui.SenzoTheme
import com.senzo.app.ui.SettingsScreen
import com.senzo.app.ui.TextDim

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            SenzoTheme {
                SenzoApp()
            }
        }
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

@Composable
private fun BottomBar(tab: Tab, onSelect: (Tab) -> Unit) {
    NavigationBar(containerColor = BackgroundElevated, tonalElevation = 0.dp) {
        tabs.forEach { spec ->
            val selected = spec.tab == tab
            NavigationBarItem(
                selected = selected,
                onClick = { onSelect(spec.tab) },
                icon = {
                    Icon(
                        imageVector = spec.icon,
                        contentDescription = spec.label,
                        modifier = Modifier.size(22.dp),
                    )
                },
                label = {
                    Text(
                        text = spec.label,
                        style = MaterialTheme.typography.labelSmall,
                    )
                },
                colors = NavigationBarItemDefaults.colors(
                    selectedIconColor = Accent,
                    selectedTextColor = Accent,
                    unselectedIconColor = TextDim,
                    unselectedTextColor = TextDim,
                    indicatorColor = Accent.copy(alpha = 0.15f),
                ),
            )
        }
    }
}