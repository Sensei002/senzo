package com.senzo.app

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** Where the app is in its own small navigation stack. */
sealed interface Screen {
    data object Login : Screen
    data object Home : Screen
    data class Details(val type: String, val id: String) : Screen
}

data class DetailsState(
    val type: String,
    val id: String,
    val loading: Boolean = true,
    val meta: Meta? = null,
    val selectedVideo: String? = null,
    val streams: List<StreamItem> = emptyList(),
    val streamsLoading: Boolean = false,
    val streamError: String? = null,
)

data class UiState(
    val screen: Screen = Screen.Login,
    val session: Session? = null,
    val signingIn: Boolean = false,
    val addons: List<AddonDescriptor> = emptyList(),
    val board: List<CatalogRow> = emptyList(),
    val boardLoading: Boolean = false,
    val boardError: String? = null,
    val searchQuery: String = "",
    val searchResults: List<MetaPreview> = emptyList(),
    val searchLoading: Boolean = false,
    val details: DetailsState? = null,
    val playback: Playback? = null,
    val message: String? = null,
)

/**
 * The Android client of the same account the desktop app signs into.
 *
 * Everything it shows comes from the installed addons of that account: the
 * board is their catalogues, details are their metadata, and playback starts
 * from their streams.
 */
class MainViewModel(application: Application) : AndroidViewModel(application) {
    private val store = SessionStore(application)
    private val api = StremioApi()
    private val addonClient = AddonClient()

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    private var searchJob: Job? = null
    private var boardJob: Job? = null

    init {
        val session = store.session()
        if (session == null) {
            _state.update { it.copy(screen = Screen.Login) }
        } else {
            val cached = store.addons()
            _state.update {
                it.copy(
                    session = session,
                    screen = Screen.Home,
                    addons = cached,
                )
            }
            if (cached.isNotEmpty()) {
                loadBoard(cached)
            }
            refreshAddons(session, cached.isEmpty())
        }
    }

    fun signIn(email: String, password: String) {
        if (email.isBlank() || password.isBlank()) {
            _state.update { it.copy(message = "Enter your Stremio email and password.") }
            return
        }
        viewModelScope.launch {
            _state.update { it.copy(signingIn = true, message = null) }
            runCatching { api.login(email.trim(), password) }
                .onSuccess { session ->
                    store.saveSession(session)
                    _state.update {
                        it.copy(
                            signingIn = false,
                            session = session,
                            screen = Screen.Home,
                            message = null,
                        )
                    }
                    refreshAddons(session, force = true)
                }
                .onFailure { error ->
                    _state.update {
                        it.copy(
                            signingIn = false,
                            message = error.message ?: "Sign-in failed.",
                        )
                    }
                }
        }
    }

    fun signOut() {
        boardJob?.cancel()
        searchJob?.cancel()
        store.clearSession()
        _state.update { UiState(screen = Screen.Login) }
    }

    fun dismissMessage() {
        _state.update { it.copy(message = null) }
    }

    fun refresh() {
        val session = _state.value.session ?: return
        refreshAddons(session, force = true)
    }

    private fun refreshAddons(session: Session, force: Boolean) {
        viewModelScope.launch {
            if (force) {
                _state.update { it.copy(boardError = null) }
            }
            runCatching { api.addons(session.authKey) }
                .onSuccess { addons ->
                    if (addons.isEmpty()) {
                        _state.update {
                            it.copy(boardError = "This account has no installed addons.")
                        }
                        return@onSuccess
                    }
                    store.saveAddons(addons)
                    _state.update { it.copy(addons = addons) }
                    loadBoard(addons)
                }
                .onFailure { error ->
                    if (_state.value.addons.isEmpty()) {
                        _state.update {
                            it.copy(
                                boardError = error.message
                                    ?: "The addon collection could not be loaded.",
                            )
                        }
                    }
                }
        }
    }

    private fun loadBoard(addons: List<AddonDescriptor>) {
        boardJob?.cancel()
        boardJob = viewModelScope.launch {
            _state.update { it.copy(boardLoading = true, boardError = null) }
            val requests = addons
                .flatMap { descriptor ->
                    descriptor.manifest.catalogs
                        .filter { it.type == "movie" || it.type == "series" }
                        .filter { it.requiredExtras.isEmpty() }
                        .take(2)
                        .map { catalog -> descriptor to catalog }
                }
                .take(MAX_BOARD_ROWS)
            val rows = coroutineScope {
                requests
                    .map { (descriptor, catalog) ->
                        async {
                            runCatching {
                                Triple(
                                    descriptor,
                                    catalog,
                                    addonClient.catalog(descriptor, catalog),
                                )
                            }.getOrNull()
                        }
                    }
                    .awaitAll()
            }.mapNotNull { result ->
                val (descriptor, catalog, items) = result ?: return@mapNotNull null
                if (items.isEmpty()) {
                    return@mapNotNull null
                }
                // A catalogue often has no title of its own; the addon name is
                // what the desktop client shows in that case as well.
                CatalogRow(
                    title = catalog.name?.takeIf { it.isNotBlank() }
                        ?: descriptor.manifest.name,
                    type = catalog.type,
                    items = items.take(MAX_ROW_ITEMS),
                )
            }.distinctBy { it.title }
            _state.update { it.copy(boardLoading = false, board = rows) }
            if (rows.isEmpty()) {
                _state.update {
                    it.copy(boardError = "No addon returned a catalogue.")
                }
            }
        }
    }

    fun search(query: String) {
        _state.update { it.copy(searchQuery = query) }
        searchJob?.cancel()
        if (query.isBlank()) {
            _state.update { it.copy(searchResults = emptyList(), searchLoading = false) }
            return
        }
        searchJob = viewModelScope.launch {
            delay(SEARCH_DEBOUNCE_MS)
            _state.update { it.copy(searchLoading = true) }
            val requests = _state.value.addons
                .flatMap { descriptor ->
                    descriptor.manifest.catalogs
                        .filter { it.supportsSearch }
                        .filter { it.type == "movie" || it.type == "series" }
                        .map { catalog -> descriptor to catalog }
                }
                .take(MAX_SEARCH_REQUESTS)
            val results = coroutineScope {
                requests
                    .map { (descriptor, catalog) ->
                        async {
                            runCatching {
                                addonClient.catalog(
                                    descriptor,
                                    catalog,
                                    mapOf("search" to query),
                                )
                            }.getOrDefault(emptyList())
                        }
                    }
                    .awaitAll()
            }.flatten()
            _state.update {
                it.copy(
                    searchLoading = false,
                    searchResults = results.distinctBy { meta -> "${meta.type}:${meta.id}" },
                )
            }
        }
    }

    fun clearSearch() {
        searchJob?.cancel()
        _state.update {
            it.copy(searchQuery = "", searchResults = emptyList(), searchLoading = false)
        }
    }

    fun openDetails(type: String, id: String) {
        _state.update {
            it.copy(
                screen = Screen.Details(type, id),
                details = DetailsState(type = type, id = id),
            )
        }
        viewModelScope.launch {
            val meta = fetchMeta(type, id)
            val firstVideo = if (type == "movie") {
                id
            } else {
                meta?.videos?.firstOrNull()?.id
            }
            _state.update { current ->
                current.copy(
                    details = current.details?.copy(
                        loading = false,
                        meta = meta,
                        selectedVideo = firstVideo,
                    ),
                )
            }
            if (firstVideo != null) {
                loadStreams(type, firstVideo)
            }
        }
    }

    fun backToBoard() {
        _state.update { it.copy(screen = Screen.Home, details = null) }
    }

    fun selectVideo(videoId: String) {
        val details = _state.value.details ?: return
        _state.update {
            it.copy(details = details.copy(selectedVideo = videoId))
        }
        loadStreams(details.type, videoId)
    }

    private suspend fun fetchMeta(type: String, id: String): Meta? {
        val addons = _state.value.addons.filter { it.manifest.serves("meta", type) }
        for (descriptor in addons) {
            val meta = runCatching { addonClient.meta(descriptor, type, id) }.getOrNull()
            if (meta != null) return meta
        }
        return runCatching { addonClient.meta(CINEMETA, type, id) }.getOrNull()
    }

    private fun loadStreams(type: String, videoId: String) {
        viewModelScope.launch {
            _state.update { current ->
                current.copy(
                    details = current.details?.copy(
                        streamsLoading = true,
                        streams = emptyList(),
                        streamError = null,
                    ),
                )
            }
            val addons = _state.value.addons.filter { it.manifest.serves("stream", type) }
            val responses = coroutineScope {
                addons
                    .map { descriptor ->
                        async {
                            descriptor.manifest.name to runCatching {
                                addonClient.streams(descriptor, type, videoId)
                            }.getOrDefault(emptyList())
                        }
                    }
                    .awaitAll()
            }
            val items = responses.flatMap { (addon, streams) ->
                streams.mapNotNull { stream -> stream.toItem(addon) }
            }
            _state.update { current ->
                current.copy(
                    details = current.details?.copy(
                        streamsLoading = false,
                        streams = items,
                        streamError = if (items.isEmpty()) {
                            "No addon offered a playable stream for this title."
                        } else {
                            null
                        },
                    ),
                )
            }
        }
    }

    fun play(item: StreamItem) {
        _state.update {
            it.copy(
                playback = Playback(
                    url = item.url,
                    title = listOf(item.name, item.detail)
                        .filter { value -> value.isNotBlank() }
                        .joinToString(" · "),
                    headers = item.headers,
                ),
            )
        }
    }

    fun stopPlayback() {
        _state.update { it.copy(playback = null) }
    }

    private companion object {
        const val MAX_BOARD_ROWS = 14
        const val MAX_ROW_ITEMS = 24
        const val MAX_SEARCH_REQUESTS = 12
        const val SEARCH_DEBOUNCE_MS = 350L

        val CINEMETA = AddonDescriptor(
            transportUrl = "https://v3-cinemeta.strem.io",
            manifest = AddonManifest(
                id = "com.linvo.cinemeta",
                name = "Cinemeta",
                types = listOf("movie", "series"),
            ),
        )
    }
}

/**
 * Flattens an addon stream into something the player can open.
 *
 * Torrent infohashes need a streaming engine; this client plays the HTTP
 * streams the addons offer directly.
 */
private fun Stream.toItem(addon: String): StreamItem? {
    val address = url?.takeIf { it.startsWith("http://") || it.startsWith("https://") }
        ?: return null
    return StreamItem(
        addon = addon,
        name = name?.takeIf { it.isNotBlank() } ?: addon,
        detail = title?.takeIf { it.isNotBlank() }
            ?: description?.takeIf { it.isNotBlank() }
            ?: "",
        url = address,
        headers = behaviorHints?.proxyHeaders?.request.orEmpty(),
    )
}
