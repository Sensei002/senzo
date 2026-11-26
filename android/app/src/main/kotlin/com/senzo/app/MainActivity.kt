package com.senzo.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.senzo.app.ui.Background
import com.senzo.app.ui.DetailsScreen
import com.senzo.app.ui.HomeScreen
import com.senzo.app.ui.LoadingBlock
import com.senzo.app.ui.LoginScreen
import com.senzo.app.ui.PlayerScreen
import com.senzo.app.ui.SenzoTheme

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
        PlayerScreen(playback = playback, onClose = viewModel::stopPlayback)
        return
    }

    when (val screen = state.screen) {
        Screen.Login -> LoginScreen(
            signingIn = state.signingIn,
            message = state.message,
            onSignIn = viewModel::signIn,
        )
        Screen.Home -> HomeScreen(
            state = state,
            onSearch = viewModel::search,
            onClearSearch = viewModel::clearSearch,
            onOpen = viewModel::openDetails,
            onRefresh = viewModel::refresh,
            onSignOut = viewModel::signOut,
            onDismissMessage = viewModel::dismissMessage,
        )
        is Screen.Details -> {
            BackHandler { viewModel.backToBoard() }
            val details = state.details
            if (details == null) {
                Box(modifier = Modifier.fillMaxSize().background(Background)) {
                    LoadingBlock("Loading details…")
                }
            } else {
                DetailsScreen(
                    state = details,
                    onBack = viewModel::backToBoard,
                    onSelectVideo = viewModel::selectVideo,
                    onPlay = viewModel::play,
                )
            }
        }
    }
}
