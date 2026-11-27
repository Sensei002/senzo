package com.senzo.app

import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody

/** Errors the UI can show as-is. */
class ApiException(message: String) : IOException(message)

/**
 * The account API: sign in and the addon collection of the signed-in account.
 */
class StremioApi(private val client: OkHttpClient = Http.client) {
    private val json = Http.json
    private val mediaType = "application/json; charset=utf-8".toMediaType()

    suspend fun login(email: String, password: String): Session = withContext(Dispatchers.IO) {
        val body = json.encodeToString(LoginRequest(email = email, password = password))
        val envelope: ApiEnvelope<LoginResult> =
            json.decodeFromString(post("$API_URL/login", body))
        envelope.error?.message?.let { throw ApiException(it) }
        val authKey = envelope.result?.authKey?.takeIf { it.isNotBlank() }
            ?: throw ApiException("This account did not return a sign-in key.")
        Session(authKey = authKey, email = envelope.result?.user?.email ?: email)
    }

    suspend fun addons(authKey: String): List<AddonDescriptor> = withContext(Dispatchers.IO) {
        val body = json.encodeToString(AddonCollectionRequest(authKey = authKey))
        val envelope: ApiEnvelope<AddonCollectionResult> =
            json.decodeFromString(post("$API_URL/addonCollectionGet", body))
        envelope.error?.message?.let { throw ApiException(it) }
        envelope.result?.addons.orEmpty()
    }

    private fun post(url: String, body: String): String {
        val request = Request.Builder()
            .url(url)
            .post(body.toRequestBody(mediaType))
            .header("Accept", "application/json")
            .build()
        client.newCall(request).execute().use { response ->
            val text = response.body?.string().orEmpty()
            if (!response.isSuccessful) {
                throw ApiException("The server answered HTTP ${response.code}.")
            }
            return text
        }
    }

    private companion object {
        const val API_URL = "https://api.strem.io/api"
    }
}

/** Addon requests: catalogues, metadata and streams. */
class AddonClient(private val client: OkHttpClient = Http.client) {
    private val json = Http.json

    suspend fun catalog(
        descriptor: AddonDescriptor,
        catalog: CatalogDescriptor,
        extras: Map<String, String> = emptyMap(),
    ): List<MetaPreview> = withContext(Dispatchers.IO) {
        val url = buildString {
            append(descriptor.transportUrl.trimEnd('/'))
            append("/catalog/")
            append(encode(catalog.type))
            append('/')
            append(encode(catalog.id))
            if (extras.isNotEmpty()) {
                append('/')
                append(extras.entries.joinToString("&") { (key, value) -> "$key=${encode(value)}" })
            }
            append(".json")
        }
        val body = get(url)
        json.decodeFromString<CatalogResponse>(body).metas
    }

    suspend fun meta(descriptor: AddonDescriptor, type: String, id: String): Meta? =
        withContext(Dispatchers.IO) {
            val url = "${descriptor.transportUrl.trimEnd('/')}/meta/${encode(type)}/${encode(id)}.json"
            json.decodeFromString<MetaResponse>(get(url)).meta
        }

    suspend fun streams(descriptor: AddonDescriptor, type: String, id: String): List<Stream> =
        withContext(Dispatchers.IO) {
            val url = "${descriptor.transportUrl.trimEnd('/')}/stream/${encode(type)}/${encode(id)}.json"
            json.decodeFromString<StreamResponse>(get(url)).streams
        }

    /**
     * Reads the manifest of an addon the viewer typed in.
     *
     * Addons are addressed by their base URL; pasting the manifest URL itself
     * works too, so the input is normalised before the request.
     */
    suspend fun manifest(input: String): Pair<String, AddonManifest> = withContext(Dispatchers.IO) {
        val base = normaliseBase(input)
        val body = get("$base/manifest.json")
        val manifest = runCatching { json.decodeFromString<AddonManifest>(body) }
            .getOrElse { throw ApiException("That address does not serve an addon manifest.") }
        if (manifest.id.isBlank() || manifest.name.isBlank()) {
            throw ApiException("That manifest is missing an id or a name.")
        }
        base to manifest
    }

    private fun normaliseBase(input: String): String {
        var url = input.trim()
        if (!url.startsWith("http://") && !url.startsWith("https://")) {
            url = "https://$url"
        }
        url = url.trimEnd('/')
        url = url.removeSuffix("/manifest.json")
        if (url.isEmpty()) throw ApiException("Enter the address of an addon.")
        return url
    }

    private fun get(url: String): String {
        val request = Request.Builder()
            .url(url)
            .header("Accept", "application/json")
            .build()
        client.newCall(request).execute().use { response ->
            if (!response.isSuccessful) {
                throw ApiException("The addon answered HTTP ${response.code}.")
            }
            return response.body?.string().orEmpty()
        }
    }

    private fun encode(value: String): String =
        java.net.URLEncoder.encode(value, "UTF-8").replace("+", "%20")
}
