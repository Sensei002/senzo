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

/** The five places the bottom bar can take the viewer. */
enum class Tab { Home, Search, Library, Addons, Settings }

/** Where the app is in its own navigation stack. */
sealed interface Screen {
    data object Login : Screen
    data object Main : Screen
    data class Details(val type: String, val id: String) : Screen
}

data class DetailsState(
    val type: String,
    val id: String,
    val loading: Boolean = true,
    val preview: MetaPreview? = null,
    val meta: Meta? = null,
    val season: Int? = null,
    val selectedVideo: Video? = null,
    val inLibrary: Boolean = false,
    val streams: List<StreamItem> = emptyList(),
    val streamsLoading: Boolean = false,
    val streamError: String? = null,
    val resume: WatchProgress? = null,
) {
    val title: String get() = meta?.name ?: preview?.name.orEmpty()
    val backdrop: String? get() = meta?.background ?: preview?.background ?: meta?.poster
    val poster: String? get() = meta?.poster ?: preview?.poster
    val bestStream: StreamItem? get() =
        streams.maxWithOrNull(compareBy({ it.qualityRank }, { it.seeds ?: 0 }))
}

data class UiState(
    val screen: Screen = Screen.Login,
    val tab: Tab = Tab.Home,
    val session: Session? = null,
    val signingIn: Boolean = false,
    val addons: List<AddonDescriptor> = emptyList(),
    val addonOrder: List<String> = emptyList(),
    val disabledAddons: Set<String> = emptySet(),
    val board: List<CatalogRow> = emptyList(),
    val hero: List<MetaPreview> = emptyList(),
    val boardLoading: Boolean = false,
    val boardError: String? = null,
    val searchQuery: String = "",
    val searchResults: List<MetaPreview> = emptyList(),
    val searchLoading: Boolean = false,
    val recentSearches: List<String> = emptyList(),
    val library: List<LibraryEntry> = emptyList(),
    val progress: List<WatchProgress> = emptyList(),
    val details: DetailsState? = null,
    val playback: Playback? = null,
    val settings: AppSettings = AppSettings(),
    val installing: Boolean = false,
    val message: String? = null,
) {
    /** Addons the viewer has not switched off, in their chosen order. */
    val enabledAddons: List<AddonDescriptor>
        get() {
            val ordered = addons.sortedWith(
                compareBy({ addonOrder.indexOf(it.transportUrl).let { index -> if (index < 0) Int.MAX_VALUE else index } }),
            )
            return ordered.filterNot { it.transportUrl in disabledAddons }
        }

    /** Resume points the board shows, newest first and not finished. */
    val continueWatching: List<WatchProgress>
        get() = progress.filter { !it.finished && it.positionMs > 30_000 }.take(12)
}

/**
 * The Android client of the same account the desktop app signs into.
 *
 * Everything it shows comes from the installed addons of that account: the
 * board is their catalogues, details are their metadata, and playback starts
 * from their streams.
 */
class MainViewModel(application: Application) : AndroidViewModel(application) {
    private val prefs = PrefsStore(application)
    private val sessions = SessionStore(application)
    private val api = StremioApi()
    private val addonClient = AddonClient()

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    /** Set when a board title was opened with the intent to start playing. */
    private var autoplayAfterLoad = false

    private var searchJob: Job? = null
    private var boardJob: Job? = null
    private var detailsJob: Job? = null

    init {
        val settings = prefs.settings()
        val library = prefs.library()
        val progress = prefs.progress()
        val searches = prefs.searches()
        val disabled = prefs.disabledAddons()
        val order = prefs.addonOrder()
        val session = sessions.session()
        _state.update {
            it.copy(
                settings = settings,
                library = library,
                progress = progress,
                recentSearches = searches,
                disabledAddons = disabled,
                addonOrder = order,
            )
        }
        if (session == null) {
            _state.update { it.copy(screen = Screen.Login) }
            return
        }
        val cached = mergeLocal(sessions.addons())
        _state.update {
            it.copy(screen = Screen.Main, session = session, addons = cached)
        }
        if (cached.isNotEmpty()) {
            loadBoard(cached)
        }
        refreshAddons(session, cached.isEmpty())
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
                    sessions.saveSession(session)
                    _state.update {
                        it.copy(signingIn = false, session = session, screen = Screen.Main)
                    }
                    refreshAddons(session, force = true)
                }
                .onFailure { error ->
                    _state.update {
                        it.copy(signingIn = false, message = error.message ?: "Sign-in failed.")
                    }
                }
        }
    }

    fun signOut() {
        boardJob?.cancel()
        searchJob?.cancel()
        detailsJob?.cancel()
        sessions.clearSession()
        _state.update {
            it.copy(
                screen = Screen.Login,
                session = null,
                addons = emptyList(),
                board = emptyList(),
                hero = emptyList(),
                details = null,
                playback = null,
                tab = Tab.Home,
            )
        }
    }

    fun dismissMessage() {
        _state.update { it.copy(message = null) }
    }

    fun selectTab(tab: Tab) {
        _state.update { it.copy(tab = tab) }
    }

    fun refresh() {
        val session = _state.value.session ?: return
        refreshAddons(session, force = true)
    }

    private fun mergeLocal(addons: List<AddonDescriptor>): List<AddonDescriptor> {
        val merged = (addons + prefs.localAddons())
            .distinctBy { it.transportUrl }
        sessions.saveAddons(merged)
        return merged
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
                            it.copy(message = "This account has no installed addons.")
                        }
                        return@onSuccess
                    }
                    val merged = mergeLocal(addons)
                    _state.update { it.copy(addons = merged) }
                    loadBoard(merged)
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
            val disabled = _state.value.disabledAddons
            val requests = addons
                .filterNot { it.transportUrl in disabled }
                .flatMap { descriptor ->
                    descriptor.manifest.catalogs
                        .filter { it.type == "movie" || it.type == "series" }
                        .filter { it.requiredExtras.isEmpty() }
                        .take(ROWS_PER_ADDON)
                        .map { catalog -> descriptor to catalog }
                }
                .take(MAX_BOARD_ROWS)
            val rows = coroutineScope {
                requests
                    .map { (descriptor, catalog) ->
                        async {
                            runCatching {
                                Triple(descriptor, catalog, addonClient.catalog(descriptor, catalog))
                            }.getOrNull()
                        }
                    }
                    .awaitAll()
            }.mapNotNull { result ->
                val (descriptor, catalog, items) = result ?: return@mapNotNull null
                if (items.isEmpty()) return@mapNotNull null
                // A catalogue often has no title of its own; the addon name is
                // what the desktop client shows in that case as well.
                CatalogRow(
                    title = catalog.name?.takeIf { it.isNotBlank() } ?: descriptor.manifest.name,
                    type = catalog.type,
                    items = items.take(MAX_ROW_ITEMS),
                )
            }.distinctBy { it.title }
            val hero = rows
                .flatMap { it.items }
                .distinctBy { it.key }
                .filter { !it.background.isNullOrBlank() || !it.poster.isNullOrBlank() }
                .take(MAX_HERO)
            _state.update {
                it.copy(boardLoading = false, board = rows, hero = hero)
            }
            if (rows.isEmpty()) {
                _state.update { it.copy(boardError = "No addon returned a catalogue.") }
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
            val results = searchCatalogs(query)
            _state.update {
                it.copy(
                    searchLoading = false,
                    searchResults = results.distinctBy { meta -> meta.key },
                )
            }
        }
    }

    /** Runs the current query at once and remembers it. */
    fun submitSearch() {
        val query = _state.value.searchQuery.trim()
        if (query.isEmpty()) return
        prefs.addSearch(query)
        _state.update { it.copy(recentSearches = prefs.searches()) }
        searchJob?.cancel()
        searchJob = viewModelScope.launch {
            _state.update { it.copy(searchLoading = true) }
            val results = searchCatalogs(query)
            _state.update {
                it.copy(
                    searchLoading = false,
                    searchResults = results.distinctBy { meta -> meta.key },
                )
            }
        }
    }

    private suspend fun searchCatalogs(query: String): List<MetaPreview> {
        val requests = _state.value.enabledAddons
            .flatMap { descriptor ->
                descriptor.manifest.catalogs
                    .filter { it.supportsSearch }
                    .filter { it.type == "movie" || it.type == "series" }
                    .map { catalog -> descriptor to catalog }
            }
            .take(MAX_SEARCH_REQUESTS)
        return coroutineScope {
            requests
                .map { (descriptor, catalog) ->
                    async {
                        runCatching {
                            addonClient.catalog(descriptor, catalog, mapOf("search" to query))
                        }.getOrDefault(emptyList())
                    }
                }
                .awaitAll()
        }.flatten()
    }

    fun clearSearch() {
        searchJob?.cancel()
        _state.update {
            it.copy(searchQuery = "", searchResults = emptyList(), searchLoading = false)
        }
    }

    fun clearRecentSearches() {
        prefs.clearSearches()
        _state.update { it.copy(recentSearches = emptyList()) }
    }

    fun openDetails(type: String, id: String, preview: MetaPreview? = null) {
        autoplayAfterLoad = false
        detailsJob?.cancel()
        _state.update {
            it.copy(
                screen = Screen.Details(type, id),
                details = DetailsState(
                    type = type,
                    id = id,
                    preview = preview,
                    inLibrary = _state.value.library.any { it.item.key == "$type:$id" },
                ),
            )
        }
        detailsJob = viewModelScope.launch {
            val meta = fetchMeta(type, id)
            val resume = _state.value.progress
                .filter { it.type == type && it.metaId == id }
                .maxByOrNull { it.updatedAt }
            val video = when {
                meta == null -> null
                type == "movie" -> null
                resume?.videoId != null -> meta.videos.firstOrNull { it.id == resume.videoId }
                    ?: meta.firstEpisode
                else -> meta.firstEpisode ?: meta.videos.firstOrNull()
            }
            _state.update { current ->
                val details = current.details ?: return@update current
                current.copy(
                    details = details.copy(
                        loading = false,
                        meta = meta,
                        selectedVideo = video,
                        season = video?.season ?: meta?.seasons?.firstOrNull(),
                        resume = resume,
                    ),
                )
            }
            if (meta != null) {
                loadStreams(type, video?.id ?: id)
            }
        }
    }

    fun back() {
        detailsJob?.cancel()
        _state.update { it.copy(screen = Screen.Main, details = null) }
    }

    /** Opens a board title and plays it as soon as its streams arrive. */
    fun playFromBoard(preview: MetaPreview) {
        openDetails(preview.type, preview.id, preview)
        autoplayAfterLoad = true
    }

    /** Re-requests the streams of the selected episode after a failure. */
    fun reloadStreams() {
        val details = _state.value.details ?: return
        loadStreams(details.type, details.selectedVideo?.id ?: details.id)
    }

    fun selectSeason(season: Int) {
        val details = _state.value.details ?: return
        val meta = details.meta ?: return
        val episodes = meta.episodes(season)
        val keep = details.selectedVideo?.takeIf { it.season == season }
        val episode = keep ?: episodes.firstOrNull()
        _state.update {
            it.copy(details = details.copy(season = season, selectedVideo = episode))
        }
        if (episode != null) {
            loadStreams(details.type, episode.id)
        }
    }

    fun selectVideo(video: Video) {
        val details = _state.value.details ?: return
        _state.update {
            it.copy(details = details.copy(selectedVideo = video, season = video.season))
        }
        loadStreams(details.type, video.id)
    }

    private suspend fun fetchMeta(type: String, id: String): Meta? {
        val addons = _state.value.enabledAddons.filter { it.manifest.serves("meta", type) }
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
            val items = streamsFor(type, videoId)
            _state.update { current ->
                val details = current.details ?: return@update current
                if (details.selectedVideo?.id != videoId && details.type == "series") {
                    return@update current
                }
                current.copy(
                    details = details.copy(
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
            if (autoplayAfterLoad) {
                autoplayAfterLoad = false
                _state.value.details?.streams?.firstOrNull()?.let { item -> play(item) }
            }
        }
    }

    private suspend fun streamsFor(type: String, videoId: String): List<StreamItem> {
        val addons = _state.value.enabledAddons.filter { it.manifest.serves("stream", type) }
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
        return if (_state.value.settings.preferHighestQuality) {
            items.sortedWith(compareByDescending<StreamItem> { it.qualityRank }.thenByDescending { it.seeds ?: 0 })
        } else {
            items
        }
    }

    fun play(item: StreamItem) {
        val details = _state.value.details ?: return
        val meta = details.meta ?: return
        startPlayback(meta, details.selectedVideo, item)
    }

    /** Plays the highest ranked stream of the selected title. */
    fun playBest() {
        val details = _state.value.details ?: return
        val item = details.streams.firstOrNull() ?: return
        play(item)
    }

    private fun startPlayback(meta: Meta, video: Video?, item: StreamItem) {
        val videoId = video?.id ?: meta.id
        val key = WatchProgress.key(meta.type, meta.id, videoId)
        val saved = _state.value.settings.resumePlayback
            .let { resume -> if (resume) _state.value.progress.firstOrNull { it.key == key } else null }
        val start = saved?.takeIf { !it.finished && it.positionMs > RESUME_MIN_MS }?.positionMs ?: 0
        val entry = WatchProgress(
            key = key,
            type = meta.type,
            metaId = meta.id,
            videoId = videoId,
            name = meta.name,
            poster = meta.poster,
            background = meta.background,
            season = video?.season,
            episode = video?.episode,
            episodeTitle = video?.title,
        )
        val next = if (meta.type == "series") nextEpisode(meta, videoId) else null
        _state.update {
            it.copy(
                playback = Playback(
                    url = item.url,
                    title = meta.name,
                    subtitle = listOfNotNull(video?.label, item.quality, item.size)
                        .joinToString("  ·  "),
                    headers = item.headers,
                    startPositionMs = start,
                    progress = entry,
                    hasNext = next != null,
                ),
            )
        }
    }

    private fun nextEpisode(meta: Meta, videoId: String): Video? {
        val videos = meta.videos
            .filter { it.season != null && it.episode != null }
            .sortedWith(compareBy({ it.season }, { it.episode }))
        val index = videos.indexOfFirst { it.id == videoId }
        return videos.getOrNull(index + 1)
    }

    /** Records where the player is, so the board can offer the resume point. */
    fun reportProgress(positionMs: Long, durationMs: Long) {
        val entry = _state.value.playback?.progress ?: return
        if (durationMs <= 0) return
        val updated = entry.copy(positionMs = positionMs, durationMs = durationMs)
        prefs.saveProgress(updated)
        _state.update { current ->
            current.copy(
                progress = prefs.progress(),
                details = current.details?.copy(
                    resume = updated,
                ),
            )
        }
    }

    fun stopPlayback(positionMs: Long = -1, durationMs: Long = -1) {
        if (positionMs >= 0 && durationMs > 0) {
            reportProgress(positionMs, durationMs)
        }
        _state.update { it.copy(playback = null) }
    }

    /** Continues with the next episode when the viewer asked for it. */
    fun playNext() {
        val playback = _state.value.playback ?: return
        val entry = playback.progress ?: return
        val meta = _state.value.details?.meta ?: return
        val next = nextEpisode(meta, entry.videoId)
        if (next == null) {
            _state.update { it.copy(playback = null, message = "This was the last episode.") }
            return
        }
        viewModelScope.launch {
            _state.update { it.copy(message = "Loading the next episode…") }
            val items = streamsFor(meta.type, next.id)
            val item = items.firstOrNull()
            if (item == null) {
                _state.update {
                    it.copy(playback = null, message = "No stream for the next episode.")
                }
                return@launch
            }
            _state.update {
                it.copy(
                    details = it.details?.copy(selectedVideo = next, season = next.season),
                    message = null,
                )
            }
            startPlayback(meta, next, item)
        }
    }

    fun forgetProgress(key: String) {
        prefs.removeProgress(key)
        _state.update { it.copy(progress = prefs.progress()) }
    }

    fun toggleLibrary() {
        val details = _state.value.details ?: return
        val meta = details.meta ?: return
        val preview = meta.preview()
        val library = prefs.library()
        val exists = library.any { it.item.key == preview.key }
        val next = if (exists) {
            library.filterNot { it.item.key == preview.key }
        } else {
            listOf(LibraryEntry(preview, System.currentTimeMillis())) + library
        }
        prefs.saveLibrary(next)
        _state.update {
            it.copy(
                library = next,
                details = it.details?.copy(inLibrary = !exists),
                message = if (exists) "Removed from your library." else "Saved to your library.",
            )
        }
    }

    fun removeFromLibrary(key: String) {
        val next = prefs.library().filterNot { it.item.key == key }
        prefs.saveLibrary(next)
        _state.update {
            it.copy(
                library = next,
                details = it.details?.copy(
                    inLibrary = it.details?.meta?.let { meta -> meta.preview().key != key } ?: false,
                ),
            )
        }
    }

    fun installAddon(url: String) {
        val trimmed = url.trim()
        if (trimmed.isEmpty()) return
        viewModelScope.launch {
            _state.update { it.copy(installing = true) }
            runCatching { addonClient.manifest(trimmed) }
                .onSuccess { (base, manifest) ->
                    val descriptor = AddonDescriptor(transportUrl = base, manifest = manifest)
                    val locals = prefs.localAddons().filterNot { it.transportUrl == base } + descriptor
                    prefs.saveLocalAddons(locals)
                    val merged = mergeLocal(_state.value.addons)
                    _state.update {
                        it.copy(
                            installing = false,
                            addons = merged,
                            message = "${manifest.name.ifBlank { "Addon" }} installed.",
                        )
                    }
                    loadBoard(merged)
                }
                .onFailure { error ->
                    _state.update {
                        it.copy(
                            installing = false,
                            message = error.message ?: "That addon could not be reached.",
                        )
                    }
                }
        }
    }

    fun removeAddon(transportUrl: String) {
        prefs.saveLocalAddons(prefs.localAddons().filterNot { it.transportUrl == transportUrl })
        val remaining = _state.value.addons.filterNot { it.transportUrl == transportUrl }
        sessions.saveAddons(remaining)
        val disabled = _state.value.disabledAddons - transportUrl
        prefs.saveDisabledAddons(disabled)
        _state.update { it.copy(addons = remaining, disabledAddons = disabled) }
        loadBoard(remaining)
    }

    fun toggleAddon(transportUrl: String, enabled: Boolean) {
        val disabled = if (enabled) {
            _state.value.disabledAddons - transportUrl
        } else {
            _state.value.disabledAddons + transportUrl
        }
        prefs.saveDisabledAddons(disabled)
        _state.update { it.copy(disabledAddons = disabled) }
        loadBoard(_state.value.addons)
    }

    fun moveAddon(transportUrl: String, up: Boolean) {
        val order = _state.value.addons.map { it.transportUrl }.toMutableList()
        val index = order.indexOf(transportUrl)
        if (index < 0) return
        val target = if (up) index - 1 else index + 1
        if (target !in order.indices) return
        val moved = order.removeAt(index)
        order.add(target, moved)
        prefs.saveAddonOrder(order)
        _state.update { it.copy(addonOrder = order) }
    }

    fun updateSettings(transform: (AppSettings) -> AppSettings) {
        val next = transform(_state.value.settings)
        prefs.saveSettings(next)
        _state.update { it.copy(settings = next) }
    }

    fun clearWatchHistory() {
        prefs.clearProgress()
        _state.update { it.copy(progress = emptyList()) }
    }

    private companion object {
        const val MAX_BOARD_ROWS = 16
        const val ROWS_PER_ADDON = 3
        const val MAX_ROW_ITEMS = 24
        const val MAX_HERO = 5
        const val MAX_SEARCH_REQUESTS = 12
        const val SEARCH_DEBOUNCE_MS = 350L
        const val RESUME_MIN_MS = 30_000L

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