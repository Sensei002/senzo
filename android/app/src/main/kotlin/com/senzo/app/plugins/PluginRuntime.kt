package com.senzo.app.plugins

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import java.net.URI
import java.util.Locale
import java.util.concurrent.TimeUnit

/** What the Plugins screen shows. */
data class PluginUiState(
    val config: PluginConfig = PluginConfig(),
    val busy: Set<String> = emptySet(),
    val error: String? = null,
)

/** A plugin repository the viewer added. */
@Serializable
data class PluginRepository(
    val url: String,
    val name: String = "",
    val version: String = "",
    val addedAt: Long = 0,
)

/** One provider a repository declared. */
@Serializable
data class PluginProvider(
    val key: String,
    val repository: String,
    val id: String,
    val name: String,
    val description: String = "",
    val version: String = "",
    val sourceUrl: String = "",
    val types: List<String> = listOf("movie", "tv"),
    val enabled: Boolean = false,
    val hasSettings: Boolean = false,
    val settings: String = "{}",
    val logo: String = "",
)

/** Everything the plugin host persists. */
@Serializable
data class PluginConfig(
    val repositories: List<PluginRepository> = emptyList(),
    val providers: List<PluginProvider> = emptyList(),
)

/** Arguments a provider script receives for one lookup. */
@Serializable
data class CallArgs(
    val tmdbId: String,
    val mediaType: String,
    val season: Int? = null,
    val episode: Int? = null,
)

/** A title to look up. */
data class ProviderRequest(
    val tmdbId: String,
    val mediaType: String,
    val season: Int? = null,
    val episode: Int? = null,
)

/** Stores the plugin configuration in the app preferences. */
class PluginStore(context: Context) {

    private val prefs = context.getSharedPreferences("senzo.plugins", Context.MODE_PRIVATE)
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }

    fun load(): PluginConfig {
        val raw = prefs.getString(KEY, null) ?: return defaultConfig()
        val config = runCatching { json.decodeFromString<PluginConfig>(raw) }.getOrNull()
            ?: return defaultConfig()
        return config
    }

    fun save(config: PluginConfig) {
        prefs.edit().putString(KEY, json.encodeToString(config)).apply()
    }

    private fun defaultConfig(): PluginConfig {
        val config = PluginConfig(
            repositories = listOf(
                PluginRepository(
                    url = DEFAULT_MANIFEST_URL,
                    name = "All-in-One Nuvio",
                    addedAt = System.currentTimeMillis(),
                ),
            ),
        )
        save(config)
        return config
    }

    private companion object {
        const val KEY = "config"
        const val DEFAULT_MANIFEST_URL =
            "https://raw.githubusercontent.com/D3adlyRocket/All-in-One-Nuvio/refs/heads/main/manifest.json"
    }
}

/** Loads repository manifests and provider scripts over HTTPS. */
class PluginRepositoryClient {

    private val json = Json { ignoreUnknownKeys = true }
    private val client = OkHttpClient.Builder()
        .connectTimeout(8, TimeUnit.SECONDS)
        .readTimeout(20, TimeUnit.SECONDS)
        .followRedirects(true)
        .build()

    fun load(manifestUrl: String): RepositoryContent {
        var content = RepositoryContent()
        var level = listOf(Level(0, manifestUrl, fetchJson(manifestUrl)))
        while (level.isNotEmpty()) {
            val next = mutableListOf<Level>()
            for (entry in level) {
                merge(content, entry.depth, entry.url, entry.value)?.let { lists ->
                    for (list in lists) {
                        val url = resolve(entry.url, list) ?: continue
                        next.add(Level(entry.depth + 1, url, fetchJson(url)))
                    }
                }
            }
            level = next
        }
        return content
    }

    fun script(url: String): String = text(url, MAX_SCRIPT_BYTES)

    fun text(url: String, limit: Int): String {
        val uri = requireHttps(url)
        val request = Request.Builder().url(uri.toString()).build()
        client.newCall(request).execute().use { response ->
            if (!response.isSuccessful) {
                throw IllegalStateException("The plugin host answered ${response.code}.")
            }
            val declared = response.header("Content-Length")?.toLongOrNull() ?: 0
            if (declared > limit) {
                throw IllegalStateException("The plugin response is larger than the allowed limit.")
            }
            val bytes = response.body?.bytes() ?: ByteArray(0)
            if (bytes.size > limit) {
                throw IllegalStateException("The plugin response is larger than the allowed limit.")
            }
            String(bytes, Charsets.UTF_8)
        }
    }

    /** Resolves a relative provider path against the document that declared it. */
    fun resolve(base: String, path: String): String? {
        if (path.isBlank()) return null
        if (path.startsWith("http://") || path.startsWith("https://")) return path
        return runCatching { URI(base).resolve(path).toString() }.getOrNull()
    }

    private fun fetchJson(url: String): JsonElement {
        val text = text(url, MAX_MANIFEST_BYTES)
        return runCatching { json.parseToJsonElement(text) }.getOrElse {
            throw IllegalStateException("Invalid plugin manifest.")
        }
    }

    private fun requireHttps(url: String): URI {
        val uri = runCatching { URI(url.trim()) }.getOrNull()
            ?: throw IllegalArgumentException("Enter a valid HTTPS plugin manifest URL.")
        if (uri.scheme?.lowercase(Locale.US) != "https") {
            throw IllegalArgumentException("Enter a valid HTTPS plugin manifest URL.")
        }
        val host = uri.host.orEmpty()
        if (host.isBlank() || PluginFetch.isBlockedHost(host)) {
            throw IllegalArgumentException("Plugin hosts must resolve to a public address.")
        }
        return uri
    }

    private data class Level(val depth: Int, val url: String, val value: JsonElement)

    /**
     * Reads one manifest document and returns the plugin lists it points at.
     * Community repositories publish a manifest with `scrapers`, an index with
     * `pluginLists`, or a bare array of provider entries.
     */
    private fun merge(
        content: RepositoryContent,
        depth: Int,
        baseUrl: String,
        value: JsonElement,
    ): List<String>? {
        return when (value) {
            is JsonArray -> {
                for (entry in value) {
                    parseScraper(entry)?.let { content.scrapers.add(baseUrl to it) }
                }
                null
            }
            is JsonObject -> {
                if (content.name.isEmpty()) content.name = value.text("name")
                if (content.version.isEmpty()) content.version = value.text("version")
                if (content.description.isEmpty()) content.description = value.text("description")
                (value["scrapers"] as? JsonArray)?.forEach { entry ->
                    parseScraper(entry)?.let { content.scrapers.add(baseUrl to it) }
                }
                if (depth >= 2) {
                    null
                } else {
                    (value["pluginLists"] as? JsonArray)
                        ?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull?.trim() }
                        ?.filter { it.isNotEmpty() }
                }
            }
            else -> null
        }
    }

    private fun parseScraper(value: JsonElement): ManifestScraper? {
        val objectValue = value as? JsonObject ?: return null
        val name = objectValue.text("name").ifEmpty { objectValue.text("internalName") }
        val filename = objectValue.text("filename").ifEmpty { objectValue.text("url") }
        if (name.isEmpty() || filename.isEmpty()) return null
        val id = objectValue.text("id").ifEmpty { objectValue.text("internalName") }.ifEmpty { name }
        val types = mutableListOf<String>()
        for (key in listOf("supportedTypes", "tvTypes")) {
            val array = objectValue[key] as? JsonArray ?: continue
            for (entry in array) {
                val kind = normalizeType((entry as? JsonPrimitive)?.contentOrNull.orEmpty())
                if (kind.isNotEmpty() && kind !in types) types.add(kind)
            }
        }
        if (types.isEmpty()) {
            types.add("movie")
            types.add("tv")
        }
        val enabled = (objectValue["enabled"] as? JsonPrimitive)?.booleanOrNull
            ?: (objectValue["status"] as? JsonPrimitive)?.intOrNull?.let { it != 0 }
            ?: true
        return ManifestScraper(
            id = id,
            name = name,
            description = objectValue.text("description"),
            version = objectValue.text("version"),
            filename = filename,
            types = types,
            enabled = enabled,
            hasSettings = (objectValue["hasSettings"] as? JsonPrimitive)?.booleanOrNull ?: false,
            logo = objectValue.text("logo").ifEmpty { objectValue.text("iconUrl") },
        )
    }

    private fun normalizeType(value: String): String = when (value.trim().lowercase(Locale.US)) {
        "movie", "movies", "film", "films" -> "movie"
        "series", "tv", "tvseries", "tv series", "tvshow", "tv shows", "show", "shows" -> "tv"
        "live", "livetv", "live tv", "channel", "channels" -> "live"
        "anime" -> "anime"
        "nsfw", "adult" -> "nsfw"
        else -> value.trim().lowercase(Locale.US)
    }

    private fun JsonObject.text(key: String): String {
        val primitive = this[key] as? JsonPrimitive ?: return ""
        return primitive.contentOrNull?.trim().orEmpty()
    }

    private companion object {
        const val MAX_MANIFEST_BYTES = 4 * 1024 * 1024
        const val MAX_SCRIPT_BYTES = 4 * 1024 * 1024
    }
}

/** What one manifest document contributed. */
class RepositoryContent {
    var name: String = ""
    var version: String = ""
    var description: String = ""
    val scrapers = mutableListOf<Pair<String, ManifestScraper>>()
}

/** One provider entry before it becomes a [PluginProvider]. */
data class ManifestScraper(
    val id: String,
    val name: String,
    val description: String,
    val version: String,
    val filename: String,
    val types: List<String>,
    val enabled: Boolean,
    val hasSettings: Boolean,
    val logo: String,
)

/** The plugin host: installs repositories and runs providers. */
class PluginRuntime(private val context: Context) {

    private val store = PluginStore(context)
    private val engine = PluginEngine(context)
    private val client = PluginRepositoryClient()
    private val scripts = java.util.Collections.synchronizedMap(mutableMapOf<String, String>())

    private var cached: PluginConfig = store.load()

    fun config(): PluginConfig = cached

    fun replace(config: PluginConfig) {
        cached = config
        store.save(config)
    }

    /** Adds or refreshes a repository and returns how many providers it has. */
    suspend fun install(manifestUrl: String): Result<Int> = withContext(Dispatchers.IO) {
        runCatching {
            val content = client.load(manifestUrl)
            if (content.scrapers.isEmpty()) {
                throw IllegalStateException("This repository declares no plugin providers.")
            }
            val current = cached
            val previous = current.providers
                .filter { it.repository == manifestUrl }
                .associateBy { it.key }
            val refreshed = content.scrapers
                .map { (baseUrl, scraper) ->
                    val key = manifestUrl + "#" + scraper.id
                    val source = client.resolve(baseUrl, scraper.filename).orEmpty()
                    val old = previous[key]
                    PluginProvider(
                        key = key,
                        repository = manifestUrl,
                        id = scraper.id,
                        name = scraper.name,
                        description = scraper.description,
                        version = scraper.version,
                        sourceUrl = source,
                        types = scraper.types,
                        enabled = old?.enabled ?: false,
                        hasSettings = scraper.hasSettings,
                        settings = old?.settings ?: "{}",
                        logo = scraper.logo,
                    )
                }
                .distinctBy { it.key }
                .sortedBy { it.name.lowercase(Locale.US) }
            val repository = PluginRepository(
                url = manifestUrl,
                name = content.name.ifBlank { current.repositories.firstOrNull { it.url == manifestUrl }?.name.orEmpty().ifBlank { manifestUrl } },
                version = content.version,
                addedAt = System.currentTimeMillis(),
            )
            val repositories = current.repositories.filterNot { it.url == manifestUrl } + repository
            replace(
                current.copy(
                    repositories = repositories.sortedBy { it.name.lowercase(Locale.US) },
                    providers = current.providers.filterNot { it.repository == manifestUrl } + refreshed,
                ),
            )
            refreshed.size
        }
    }

    fun removeRepository(url: String) {
        val current = cached
        replace(
            current.copy(
                repositories = current.repositories.filterNot { it.url == url },
                providers = current.providers.filterNot { it.repository == url },
            ),
        )
    }

    fun setEnabled(key: String, enabled: Boolean) {
        val current = cached
        replace(
            current.copy(
                providers = current.providers.map { provider ->
                    if (provider.key == key) provider.copy(enabled = enabled) else provider
                },
            ),
        )
    }

    fun setAllEnabled(repository: String, enabled: Boolean) {
        val current = cached
        replace(
            current.copy(
                providers = current.providers.map { provider ->
                    if (provider.repository == repository) provider.copy(enabled = enabled) else provider
                },
            ),
        )
    }

    fun setSettings(key: String, settings: String) {
        val current = cached
        replace(
            current.copy(
                providers = current.providers.map { provider ->
                    if (provider.key == key) provider.copy(settings = settings) else provider
                },
            ),
        )
    }

    /** Runs every enabled provider for [request] and collects their streams. */
    suspend fun streams(request: ProviderRequest): List<PluginStream> = withContext(Dispatchers.IO) {
        val active = cached.providers.filter { provider ->
            provider.enabled && provider.sourceUrl.isNotEmpty() && request.mediaType in provider.types
        }
        active.chunked(CONCURRENCY).flatMap { chunk ->
            coroutineScope {
                chunk.map { provider ->
                    async(Dispatchers.IO) { runProvider(provider, request) }
                }.awaitAll()
            }.flatten()
        }
    }

    private suspend fun runProvider(provider: PluginProvider, request: ProviderRequest): List<PluginStream> {
        val script = try {
            scripts[provider.sourceUrl] ?: client.script(provider.sourceUrl).also { text ->
                if (scripts.size > MAX_CACHED_SCRIPTS) scripts.clear()
                scripts[provider.sourceUrl] = text
            }
        } catch (error: Throwable) {
            return emptyList()
        }
        val outcome = engine.execute(provider, script, request)
        return outcome.streams.map { stream -> stream.copy(provider = provider.name) }
    }

    private companion object {
        const val CONCURRENCY = 4
        const val MAX_CACHED_SCRIPTS = 32
    }
}
