package com.senzo.app

import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import okhttp3.Request

/** How the directory is ordered. Rising is the directory's own trending feed. */
enum class CommunitySort(val label: String) {
    Rising("Trending"),
    Stars("Top rated"),
    Newest("Newest"),
}

/** One addon of the community directory, ready to show and to install. */
@Serializable
data class CommunityAddon(
    val name: String = "",
    val description: String = "",
    val manifestUrl: String = "",
    val logo: String? = null,
    val version: String = "",
    val types: List<String> = emptyList(),
    val categories: List<String> = emptyList(),
    val stars: Long = 0,
    val adult: Boolean = false,
    val p2p: Boolean = false,
    val configurable: Boolean = false,
    val configurationRequired: Boolean = false,
    val configurationUrl: String? = null,
) {
    /** The base URL an installed descriptor is stored under. */
    val baseUrl: String
        get() = manifestUrl.removeSuffix("/manifest.json").trimEnd('/')

    /** What the addon serves, named the way the viewer reads it. */
    val kind: String
        get() = when {
            types.isEmpty() -> "Addon"
            types.size == 1 -> types.first().replaceFirstChar { it.uppercase() }
            else -> "${types.size} media types"
        }
}

@Serializable
data class CommunityPage(
    val addons: List<CommunityAddon> = emptyList(),
    val hasNextPage: Boolean = false,
)

/**
 * The community addon directory the desktop client browses.
 *
 * It lists addons with the manifest URL they install from, so installing is
 * the same operation as pasting that URL by hand: fetch the manifest, keep the
 * descriptor. Nothing here executes anything; an addon stays a web service.
 */
class CommunityClient(private val client: okhttp3.OkHttpClient = Http.client) {
    private val json = Http.json

    suspend fun browse(
        sort: CommunitySort,
        query: String,
        page: Int,
    ): CommunityPage = withContext(Dispatchers.IO) {
        val trimmed = query.trim()
        val url = buildString {
            append(BASE)
            if (sort == CommunitySort.Rising) {
                // The trending feed has no search of its own, so a query is
                // answered from a larger slice and filtered below.
                append("rising?limit=")
                append(if (trimmed.isEmpty()) PAGE_SIZE else SEARCH_LIMIT)
            } else {
                append("addons?page=")
                append(page.coerceAtLeast(1))
                append("&limit=")
                append(PAGE_SIZE)
                append("&sort_by=")
                append(if (sort == CommunitySort.Stars) "stars" else "createdAt")
                append("&order=desc")
                if (trimmed.isNotEmpty()) {
                    append("&search=")
                    append(encode(trimmed))
                }
            }
            append("&nsfw=exclude")
        }
        val body = get(url)
        val parsed = runCatching { json.decodeFromString<ApiPage>(body) }
            .getOrElse { throw ApiException("The addon directory answered with something unexpected.") }
        val addons = parsed.addons
            .mapNotNull { normalize(it) }
            .filter { addon ->
                trimmed.isEmpty() ||
                    addon.name.contains(trimmed, ignoreCase = true) ||
                    addon.description.contains(trimmed, ignoreCase = true)
            }
        CommunityPage(
            addons = addons,
            // The trending feed is a single page by design. The paged feeds
            // only report pagination once past the first page, so a full page
            // counts as "there is more" until the server says otherwise.
            hasNextPage = sort != CommunitySort.Rising &&
                (parsed.pagination?.hasNextPage ?: (addons.size >= PAGE_SIZE)),
        )
    }

    private fun normalize(api: ApiAddon): CommunityAddon? {
        val url = api.manifestUrl.trim()
        if (!url.startsWith("https://")) return null
        if (api.manifest.name.isBlank()) return null
        val adult = api.manifest.behaviorHints.adult || api.categories.any { it.slug == "nsfw" }
        if (adult) return null
        val p2p = api.manifest.behaviorHints.p2p || api.categories.any { it.slug == "torrents" }
        val configurationUrl = api.configureUrl?.takeIf { candidate ->
            candidate.startsWith("https://") &&
                runCatching { java.net.URI(candidate).host == java.net.URI(url).host }.getOrDefault(false)
        }
        return CommunityAddon(
            name = api.manifest.name.trim(),
            description = api.manifest.description.trim(),
            manifestUrl = url,
            logo = api.manifest.logo,
            version = api.manifest.version.trim(),
            types = api.manifest.types,
            categories = api.categories.mapNotNull { it.name.trim().takeIf(String::isNotEmpty) },
            stars = api.stars,
            p2p = p2p,
            configurable = api.manifest.behaviorHints.configurable && configurationUrl != null,
            configurationRequired = api.manifest.behaviorHints.configurationRequired,
            configurationUrl = configurationUrl,
        )
    }

    private fun get(url: String): String {
        val request = Request.Builder().url(url).header("Accept", "application/json").build()
        client.newCall(request).execute().use { response ->
            if (!response.isSuccessful) {
                throw IOException("The addon directory answered HTTP ${response.code}.")
            }
            return response.body?.string().orEmpty()
        }
    }

    private fun encode(value: String): String =
        java.net.URLEncoder.encode(value, "UTF-8").replace("+", "%20")

    private companion object {
        const val BASE = "https://stremio-addons.net/api/v0/"
        const val PAGE_SIZE = 25
        const val SEARCH_LIMIT = 100
    }
}

// The wire format, which names almost everything in camelCase.
@Serializable
private data class ApiPage(
    val addons: List<ApiAddon> = emptyList(),
    val pagination: ApiPagination? = null,
)

@Serializable
private data class ApiPagination(
    @SerialName("hasNextPage") val hasNextPage: Boolean = false,
)

@Serializable
private data class ApiAddon(
    val manifestUrl: String = "",
    val manifest: ApiManifest = ApiManifest(),
    val stars: Long = 0,
    val categories: List<ApiCategory> = emptyList(),
    val configureUrl: String? = null,
)

@Serializable
private data class ApiManifest(
    val name: String = "",
    val description: String = "",
    val version: String = "",
    val logo: String? = null,
    val types: List<String> = emptyList(),
    val behaviorHints: ApiHints = ApiHints(),
)

@Serializable
private data class ApiHints(
    val configurable: Boolean = false,
    val configurationRequired: Boolean = false,
    val adult: Boolean = false,
    val p2p: Boolean = false,
)

@Serializable
private data class ApiCategory(
    val name: String = "",
    val slug: String = "",
)
