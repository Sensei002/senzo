package com.senzo.app.plugins

import kotlinx.serialization.Serializable
import okhttp3.Cookie
import okhttp3.CookieJar
import okhttp3.HttpUrl
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import java.net.InetAddress
import java.net.URI
import java.util.Base64
import java.util.Locale
import java.util.concurrent.TimeUnit

/** One request from a provider script, in the desktop runtime wire format. */
@Serializable
data class FetchRequest(
    val url: String = "",
    val method: String = "",
    val headers: Map<String, String> = emptyMap(),
    val bodyKind: String = "",
    val body: String = "",
    val followRedirects: Boolean = true,
)

/** What the provider script receives back. Shape matches the desktop engine. */
@Serializable
data class FetchResponse(
    val ok: Boolean = false,
    val status: Int = 0,
    val statusText: String = "",
    val url: String = "",
    val redirected: Boolean = false,
    val headers: Map<String, String> = emptyMap(),
    val body: String = "",
    val bodyBase64: String = "",
    val error: String? = null,
)

/**
 * Blocking HTTP bridge for provider scripts.
 *
 * Redirects are resolved by hand so a public URL cannot bounce into the local
 * network, TMDB calls are rewritten to the SENZO gateway, and bodies are
 * size-capped before they reach the script runtime.
 */
class PluginFetch {

    private val cookies = MemoryCookieJar()
    private val client = OkHttpClient.Builder()
        .connectTimeout(CONNECT_SECONDS, TimeUnit.SECONDS)
        .readTimeout(REQUEST_SECONDS, TimeUnit.SECONDS)
        .writeTimeout(REQUEST_SECONDS, TimeUnit.SECONDS)
        .followRedirects(false)
        .followSslRedirects(false)
        .cookieJar(cookies)
        .build()

    fun fetch(request: FetchRequest): FetchResponse {
        var current = runCatching { URI(request.url.trim()) }.getOrNull()
            ?: return failure("The provider requested an invalid URL.")
        if (!isHttp(current)) return failure("Provider requests must use HTTP or HTTPS.")
        if (current.host.isNullOrBlank() || current.userInfo != null) {
            return failure("The provider requested an invalid URL.")
        }
        if (isBlockedHost(current.host.orEmpty())) {
            return failure("Provider requests to private hosts are blocked.")
        }
        val method = request.method.trim().takeIf { it.isNotEmpty() }?.uppercase(Locale.US) ?: "GET"
        if (method !in ALLOWED_METHODS) return failure("That HTTP method is not allowed.")
        val body = when (request.bodyKind) {
            "base64" -> runCatching { Base64.getDecoder().decode(request.body) }.getOrNull()
                ?: return failure("The provider sent an invalid request body.")
            "text" -> request.body.toByteArray()
            else -> null
        }
        var redirected = false
        var target = current
        repeat(MAX_REDIRECTS + 1) { hop ->
            val gatewayUrl = rewriteTmdb(target)
            val rewritten = gatewayUrl != null
            if (gatewayUrl != null) target = gatewayUrl
            if (isBlockedHost(target.host.orEmpty())) {
                return failure("Provider requests to private hosts are blocked.")
            }
            if (!addressIsPublic(target.host.orEmpty())) {
                return failure("Provider requests to private addresses are blocked.")
            }
            val builder = Request.Builder().url(target.toString())
            for ((name, value) in request.headers) {
                val lower = name.lowercase(Locale.US)
                if (rewritten && (lower == "authorization" || lower == "x-api-key")) continue
                if (lower == "host" || lower == "content-length" || lower == "connection") continue
                runCatching { builder.header(name, value) }
            }
            val payload = body?.toRequestBody("application/octet-stream".toMediaTypeOrNull())
                ?: if (method in BODY_METHODS) ByteArray(0).toRequestBody(null) else null
            builder.method(method, payload)
            val response = try {
                client.newCall(builder.build()).execute()
            } catch (error: Throwable) {
                return failure(messageFor(error))
            }
            response.use { answer ->
                if (answer.isRedirect) {
                    if (!request.followRedirects) {
                        return read(answer, target.toString(), redirected)
                    }
                    val location = answer.header("Location")
                        ?: return failure("The provider redirect was missing its target.")
                    val next = runCatching { target.resolve(location) }.getOrNull()
                        ?: return failure("The provider redirect was invalid.")
                    if (!isHttp(next)) {
                        return failure("The provider redirected to an unsupported URL.")
                    }
                    if (isBlockedHost(next.host.orEmpty()) || !addressIsPublic(next.host.orEmpty())) {
                        return failure("The provider redirected to a private address.")
                    }
                    if (hop == MAX_REDIRECTS) {
                        return failure("The provider redirected too many times.")
                    }
                    target = next
                    redirected = true
                } else {
                    return read(answer, target.toString(), redirected)
                }
            }
        }
        return failure("The provider redirected too many times.")
    }

    private fun read(response: Response, finalUrl: String, redirected: Boolean): FetchResponse {
        val length = response.header("Content-Length")?.toLongOrNull() ?: 0
        if (length > MAX_BYTES) {
            return failure("The provider response is larger than the allowed limit.")
        }
        val bytes = try {
            response.body?.bytes() ?: ByteArray(0)
        } catch (error: Throwable) {
            return failure("Could not read the provider response.")
        }
        if (bytes.size > MAX_BYTES) {
            return failure("The provider response is larger than the allowed limit.")
        }
        val headers = response.headers.names().associate { name ->
            name.lowercase(Locale.US) to response.headers.values(name).joinToString(", ")
        }
        return FetchResponse(
            ok = response.isSuccessful,
            status = response.code,
            statusText = response.message,
            url = finalUrl,
            redirected = redirected,
            headers = headers,
            body = String(bytes, Charsets.UTF_8),
            bodyBase64 = Base64.getEncoder().encodeToString(bytes),
        )
    }

    private fun failure(message: String) = FetchResponse(error = message, url = "")

    private fun messageFor(error: Throwable): String = when (error) {
        is java.net.SocketTimeoutException -> "The provider request timed out."
        is java.net.UnknownHostException -> "The provider host could not be reached."
        is java.net.ConnectException -> "The provider host could not be reached."
        else -> "The provider request failed."
    }

    /** Metadata-only rewrite: provider scripts assume a real TMDB key. */
    private fun rewriteTmdb(url: URI): URI? {
        if (url.host != TMDB_HOST) return null
        val path = url.path.orEmpty()
        if (!path.startsWith("/3")) return null
        val rest = path.removePrefix("/3")
        if (rest.isNotEmpty() && !rest.startsWith("/")) return null
        val kept = url.query.orEmpty()
            .split("&")
            .filter { it.isNotEmpty() }
            .filterNot { it.startsWith("api_key=") || it.startsWith("append_to_response=") }
        val query = if (kept.isEmpty()) "" else "?" + kept.joinToString("&")
        return runCatching {
            URI(
                "https",
                "senzo-api.vercel.app",
                "/api/tmdb$rest",
                query.removePrefix("?").ifEmpty { null },
                null,
            )
        }.getOrNull()
    }

    private fun isHttp(url: URI): Boolean {
        val scheme = url.scheme?.lowercase(Locale.US) ?: return false
        return scheme == "http" || scheme == "https"
    }

    private fun addressIsPublic(host: String): Boolean {
        if (host.isBlank()) return false
        val literal = runCatching { InetAddress.getByName(host) }.getOrNull()
        val addresses = if (literal != null) {
            listOf(literal)
        } else {
            runCatching { InetAddress.getAllByName(host).toList() }.getOrNull() ?: return false
        }
        return addresses.isNotEmpty() && addresses.none { isBlockedAddress(it) }
    }

    private class MemoryCookieJar : CookieJar {
        private val stored = mutableListOf<Cookie>()

        @Synchronized
        override fun saveFromResponse(url: HttpUrl, cookies: List<Cookie>) {
            stored.removeAll { old -> cookies.any { it.name == old.name && it.domain == old.domain } }
            stored.addAll(cookies)
            if (stored.size > 512) stored.subList(0, stored.size - 512).clear()
        }

        @Synchronized
        override fun loadForRequest(url: HttpUrl): List<Cookie> =
            stored.filter { cookie ->
                val domain = url.host
                domain == cookie.domain || domain.endsWith("." + cookie.domain.removePrefix("."))
            }
    }

    companion object {
        private const val CONNECT_SECONDS = 8L
        private const val REQUEST_SECONDS = 30L
        private const val MAX_REDIRECTS = 6
        private const val MAX_BYTES = 12L * 1024 * 1024
        private const val TMDB_HOST = "api.themoviedb.org"
        private val ALLOWED_METHODS = setOf("GET", "POST", "PUT", "DELETE", "PATCH", "HEAD")
        private val BODY_METHODS = setOf("POST", "PUT", "PATCH")

        fun isBlockedHost(host: String): Boolean {
            val value = host.trimEnd('.').lowercase(Locale.US)
            if (value.isEmpty()) return true
            if (value == "localhost" || value.endsWith(".localhost")) return true
            if (value.endsWith(".local") || value.endsWith(".internal")) return true
            if (value.endsWith(".home.arpa")) return true
            val literal = runCatching { InetAddress.getByName(value) }.getOrNull()
            return literal != null && isBlockedAddress(literal)
        }

        fun isBlockedAddress(address: InetAddress): Boolean {
            if (address.isAnyLocalAddress || address.isLoopbackAddress) return true
            if (address.isLinkLocalAddress || address.isSiteLocalAddress) return true
            if (address.isMulticastAddress) return true
            val bytes = address.address
            if (bytes.size == 4) {
                val first = bytes[0].toInt() and 0xFF
                val second = bytes[1].toInt() and 0xFF
                val third = bytes[2].toInt() and 0xFF
                if (first == 0) return true
                if (first == 100 && second in 64..127) return true
                if (first == 192 && second == 0 && third == 0) return true
                if (first == 198 && (second == 18 || second == 19)) return true
                if (first == 198 && second == 51 && third == 100) return true
                if (first == 203 && second == 0 && third == 113) return true
            }
            return false
        }
    }
}
