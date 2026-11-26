package com.senzo.app

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * The Stremio addon protocol, from the client side.
 *
 * The desktop application gets these types from stremio-core. The Android
 * client speaks the same protocol directly, so addons that work on the desktop
 * work here without a second implementation of each addon.
 */
@Serializable
data class AddonManifest(
    val id: String = "",
    val name: String = "",
    val version: String? = null,
    val description: String? = null,
    val logo: String? = null,
    val background: String? = null,
    val types: List<String> = emptyList(),
    val resources: List<JsonElement> = emptyList(),
    val catalogs: List<CatalogDescriptor> = emptyList(),
) {
    /** Resource names this manifest serves, e.g. `catalog`, `meta`, `stream`. */
    val resourceNames: Set<String>
        get() = resources.mapNotNull { it.resourceName() }.toSet()

    /** Whether the manifest serves [resource] for [type]. */
    fun serves(resource: String, type: String): Boolean {
        if (resource !in resourceNames) return false
        if (types.isNotEmpty() && type !in types) return false
        // A resource may also be declared as an object that narrows its types;
        // a plain string means every type the manifest supports.
        val narrowed = resources
            .filter { it.resourceName() == resource }
            .mapNotNull { it.resourceTypes() }
        return narrowed.isEmpty() || narrowed.any { type in it }
    }
}

private fun JsonElement.resourceName(): String? =
    runCatching { jsonPrimitive.contentOrNull }.getOrNull()
        ?: runCatching { jsonObject["name"]?.jsonPrimitive?.contentOrNull }.getOrNull()

private fun JsonElement.resourceTypes(): List<String>? =
    runCatching {
        jsonObject["types"]?.jsonArray?.mapNotNull { it.jsonPrimitive.contentOrNull }
    }.getOrNull()

@Serializable
data class CatalogDescriptor(
    val type: String = "",
    val id: String = "",
    val name: String? = null,
    val extra: List<ExtraDescriptor> = emptyList(),
    val extraRequired: List<String> = emptyList(),
    val extraSupported: List<String> = emptyList(),
    val genres: List<String>? = null,
) {
    /** Extras a request must supply; those catalogues need a UI of their own. */
    val requiredExtras: List<String>
        get() = (extraRequired + extra.filter { it.isRequired == true }.map { it.name })
            .filter { it.isNotBlank() }
            .distinct()

    val supportsSearch: Boolean
        get() = extraSupported.contains("search") || extra.any { it.name == "search" }

    /** Whether the catalogue can be paged with `skip`, which the board uses. */
    val supportsSkip: Boolean
        get() = extraSupported.contains("skip") || extra.any { it.name == "skip" }
}

@Serializable
data class ExtraDescriptor(
    val name: String = "",
    val isRequired: Boolean? = null,
    val options: List<String>? = null,
)

@Serializable
data class AddonDescriptor(
    val transportUrl: String = "",
    val transportName: String? = null,
    val manifest: AddonManifest = AddonManifest(),
)

@Serializable
data class MetaPreview(
    val id: String = "",
    val type: String = "",
    val name: String = "",
    val poster: String? = null,
    val posterShape: String? = null,
    val background: String? = null,
    val logo: String? = null,
    val description: String? = null,
    val releaseInfo: String? = null,
    val imdbRating: String? = null,
    val runtime: String? = null,
    val genres: List<String> = emptyList(),
) {
    val key: String get() = "$type:$id"

    /** The year a catalogue entry belongs to, when the addon provides one. */
    val year: String?
        get() = releaseInfo
            ?.let { info -> Regex("\\b(19|20)\\d{2}\\b").find(info)?.value }
}

@Serializable
data class Meta(
    val id: String = "",
    val type: String = "",
    val name: String = "",
    val poster: String? = null,
    val background: String? = null,
    val logo: String? = null,
    val description: String? = null,
    val releaseInfo: String? = null,
    val imdbRating: String? = null,
    val runtime: String? = null,
    val genres: List<String> = emptyList(),
    val videos: List<Video> = emptyList(),
    val director: List<String> = emptyList(),
    val cast: List<String> = emptyList(),
) {
    val seasons: List<Int>
        get() = videos.mapNotNull { it.season }.distinct().sorted()

    fun episodes(season: Int): List<Video> =
        videos.filter { it.season == season }.sortedBy { it.episode ?: 0 }

    /** The first episode that is not obviously a special or a trailer. */
    val firstEpisode: Video?
        get() = videos
            .filter { it.season != null && it.episode != null }
            .sortedWith(compareBy({ it.season }, { it.episode }))
            .firstOrNull()

    fun preview(): MetaPreview = MetaPreview(
        id = id,
        type = type,
        name = name,
        poster = poster,
        background = background,
        logo = logo,
        description = description,
        releaseInfo = releaseInfo,
        imdbRating = imdbRating,
        runtime = runtime,
        genres = genres,
    )
}

@Serializable
data class Video(
    val id: String = "",
    val title: String? = null,
    val season: Int? = null,
    val episode: Int? = null,
    val thumbnail: String? = null,
    val released: String? = null,
    val overview: String? = null,
) {
    val label: String
        get() = when {
            season != null && episode != null -> "S${season}E$episode" + (title?.let { " · $it" } ?: "")
            !title.isNullOrBlank() -> title
            else -> id
        }

    val shortLabel: String
        get() = when {
            season != null && episode != null -> "E$episode"
            !title.isNullOrBlank() -> title
            else -> id
        }
}

@Serializable
data class Stream(
    val name: String? = null,
    val title: String? = null,
    val description: String? = null,
    val url: String? = null,
    val externalUrl: String? = null,
    val infoHash: String? = null,
    val fileIdx: Int? = null,
    /** Trackers of the swarm, when an addon sends them with the hash. */
    val sources: List<String> = emptyList(),
    val behaviorHints: BehaviorHints? = null,
)

@Serializable
data class BehaviorHints(
    val notWebReady: Boolean? = null,
    val bingeGroup: String? = null,
    val proxyHeaders: ProxyHeaders? = null,
)

@Serializable
data class ProxyHeaders(val request: Map<String, String> = emptyMap())

@Serializable
data class CatalogResponse(val metas: List<MetaPreview> = emptyList())

@Serializable
data class MetaResponse(val meta: Meta? = null)

@Serializable
data class StreamResponse(val streams: List<Stream> = emptyList())

@Serializable
data class LoginRequest(
    val email: String,
    val password: String,
    val authKey: String? = null,
    val type: String = "Login",
)

@Serializable
data class LoginResult(val authKey: String? = null, val user: UserProfile? = null)

@Serializable
data class UserProfile(
    val email: String? = null,
    val fullname: String? = null,
    val avatar: String? = null,
)

@Serializable
data class AddonCollectionRequest(
    val type: String = "AddonCollectionGet",
    val authKey: String,
    val update: Boolean = true,
)

@Serializable
data class AddonCollectionResult(val addons: List<AddonDescriptor> = emptyList())

@Serializable
data class ApiEnvelope<T>(
    val result: T? = null,
    val error: ApiErrorEnvelope? = null,
)

@Serializable
data class ApiErrorEnvelope(
    val message: String? = null,
    val code: Int? = null,
    val retryAfter: Int? = null,
)

/** A signed-in Stremio account. */
data class Session(val authKey: String, val email: String)

/** A playable stream, flattened from an addon response. */
data class StreamItem(
    val addon: String,
    val name: String,
    val detail: String,
    val url: String,
    val headers: Map<String, String> = emptyMap(),
    val quality: String? = null,
    val size: String? = null,
    val seeds: Int? = null,
    /** Set when the stream is a torrent the local engine has to fetch. */
    val torrent: String? = null,
) {
    val qualityRank: Int
        get() = when (quality) {
            "4K" -> 4
            "1080p" -> 3
            "720p" -> 2
            "480p" -> 1
            else -> 0
        }

    val badge: String
        get() = when {
            torrent != null -> quality ?: "P2P"
            quality != null -> quality
            else -> "HTTP"
        }
}

/**
 * What the player needs to start a stream.
 *
 * A torrent opens the player before it has an address: [url] stays null while
 * the engine connects, [status] carries the engine's own words, and [torrent]
 * keeps the magnet so a failure can be retried without leaving the player.
 * Once it plays, [stats] keeps the swarm numbers the buffering spinner shows.
 */
data class Playback(
    val url: String? = null,
    val title: String,
    val subtitle: String = "",
    /** The year the title came out, shown under its name while it plays. */
    val year: String? = null,
    val headers: Map<String, String> = emptyMap(),
    val startPositionMs: Long = 0,
    val progress: WatchProgress? = null,
    val hasNext: Boolean = false,
    val status: String? = null,
    val failure: String? = null,
    val torrent: String? = null,
    val stats: String? = null,
)

/** One row of the board: a catalogue served by an installed addon. */
data class CatalogRow(
    val title: String,
    val type: String,
    val items: List<MetaPreview>,
)

/** Where the viewer stopped in a title, as the board and the player need it. */
@Serializable
data class WatchProgress(
    val key: String,
    val type: String,
    val metaId: String,
    val videoId: String,
    val name: String,
    val poster: String? = null,
    val background: String? = null,
    val season: Int? = null,
    val episode: Int? = null,
    val episodeTitle: String? = null,
    val positionMs: Long = 0,
    val durationMs: Long = 0,
    val updatedAt: Long = 0,
) {
    val fraction: Float
        get() = if (durationMs <= 0) 0f else (positionMs.toFloat() / durationMs).coerceIn(0f, 1f)

    /** Near the end counts as finished, the same window the desktop uses. */
    val finished: Boolean
        get() = durationMs > 0 && positionMs >= durationMs - FINISH_MARGIN_MS

    val episodeLabel: String?
        get() = when {
            season != null && episode != null -> "S$season · E$episode"
            else -> null
        }

    companion object {
        const val FINISH_MARGIN_MS = 45_000L

        fun key(type: String, metaId: String, videoId: String): String =
            "$type:$metaId:$videoId"
    }
}

@Serializable
data class LibraryEntry(val item: MetaPreview, val addedAt: Long = 0)

/** The playback engine. Media3 uses the phone's decoders; libmpv is mpv. */
enum class PlayerEngine { Media3, Libmpv }

/** Preferences that change behaviour rather than appearance. */
@Serializable
data class AppSettings(
    val autoplayNext: Boolean = true,
    val resumePlayback: Boolean = true,
    val preferHighestQuality: Boolean = true,
    val subtitleSize: Int = 1,
    /** "media3" or "libmpv"; older saves fall back to media3. */
    val playerEngine: String = "media3",
)

/** The engine a saved preference asks for. */
val AppSettings.engine: PlayerEngine
    get() = if (playerEngine == "libmpv") PlayerEngine.Libmpv else PlayerEngine.Media3

private val QUALITY_PATTERN = Regex("(?i)\\b(2160p|1080p|720p|480p|360p|4k|uhd)\\b")
private val SIZE_PATTERN = Regex("(?i)(\\d+(?:\\.\\d+)?)\\s?(gb|mb)\\b")
private val SEED_PATTERN = Regex("(?i)(?:👤|seeds?)\\s*:?\\s*(\\d+)")

/** Builds the magnet URI the local torrent engine opens. */
private fun Stream.magnet(hash: String): String = buildString {
    append("magnet:?xt=urn:btih:").append(hash)
    name?.trim()?.takeIf { it.isNotEmpty() }?.let { title ->
        append("&dn=").append(java.net.URLEncoder.encode(title, "UTF-8"))
    }
    sources
        .filter { it.startsWith("tracker:") }
        .forEach { tracker ->
            append("&tr=").append(
                java.net.URLEncoder.encode(tracker.removePrefix("tracker:"), "UTF-8"),
            )
        }
}

/**
 * Flattens an addon stream into something the player can open.
 *
 * Torrent infohashes become magnet links the local engine opens, so Torrentio
 * and the plugins play without a debrid account. The label an addon advertises
 * is kept on the item either way.
 */
fun Stream.toItem(addon: String): StreamItem? {
    val address = url?.takeIf { it.startsWith("http://") || it.startsWith("https://") }
    val hash = infoHash?.trim()?.takeIf { it.length >= 32 }
    if (address == null && hash == null) return null
    val text = listOfNotNull(name, title, description).joinToString(" ")
    val quality = QUALITY_PATTERN.find(text)?.value?.let { value ->
        when (value.lowercase()) {
            "4k", "uhd", "2160p" -> "4K"
            else -> value.lowercase()
        }
    }
    val size = SIZE_PATTERN.find(text)?.let { match ->
        val amount = match.groupValues[1]
        val unit = match.groupValues[2].uppercase()
        "$amount $unit"
    }
    val seeds = SEED_PATTERN.find(text)?.groupValues?.getOrNull(1)?.toIntOrNull()
    return StreamItem(
        addon = addon,
        name = name?.takeIf { it.isNotBlank() } ?: addon,
        detail = title?.takeIf { it.isNotBlank() }
            ?: description?.takeIf { it.isNotBlank() }
            ?: "",
        url = address ?: "",
        headers = behaviorHints?.proxyHeaders?.request.orEmpty(),
        quality = quality,
        size = size,
        seeds = seeds,
        torrent = if (address == null && hash != null) magnet(hash) else null,
    )
}