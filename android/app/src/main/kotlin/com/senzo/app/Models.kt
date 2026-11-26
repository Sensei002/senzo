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
    val description: String? = null,
    val releaseInfo: String? = null,
    val imdbRating: String? = null,
)

@Serializable
data class Meta(
    val id: String = "",
    val type: String = "",
    val name: String = "",
    val poster: String? = null,
    val background: String? = null,
    val description: String? = null,
    val releaseInfo: String? = null,
    val imdbRating: String? = null,
    val runtime: String? = null,
    val genres: List<String> = emptyList(),
    val videos: List<Video> = emptyList(),
)

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
            season != null && episode != null -> "S${season}E${episode} · ${title ?: "Episode $episode"}"
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
)

/** What the player needs to start a stream. */
data class Playback(
    val url: String,
    val title: String,
    val headers: Map<String, String> = emptyMap(),
)

/** One row of the board: a catalogue served by an installed addon. */
data class CatalogRow(
    val title: String,
    val type: String,
    val items: List<MetaPreview>,
)
