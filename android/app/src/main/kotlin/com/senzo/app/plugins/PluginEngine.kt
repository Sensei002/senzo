package com.senzo.app.plugins

import android.content.Context
import com.whl.quickjs.android.QuickJSLoader
import com.whl.quickjs.wrapper.JSCallFunction
import com.whl.quickjs.wrapper.QuickJSContext
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject

/** What one provider run produced. */
data class PluginOutcome(
    val streams: List<PluginStream> = emptyList(),
    val error: String? = null,
    val logs: List<String> = emptyList(),
)

/** One stream a provider script returned. */
data class PluginStream(
    val name: String,
    val description: String,
    val url: String?,
    val infoHash: String?,
    val fileIdx: Int?,
    val headers: Map<String, String>,
    val seeders: Int?,
    val provider: String = "",
)

/**
 * Runs Nuvio provider scripts on the device.
 *
 * The desktop client hosts these scripts with QuickJS; Android uses the same
 * engine through a JNI wrapper, so the provider corpus runs unchanged. Every
 * bridge the scripts can reach is deliberately narrow: HTTP, sleeping timers,
 * URL parsing, text encoding, crypto, and an HTML selector facade.
 */
class PluginEngine(private val context: Context) {

    private val fetch = PluginFetch()
    private val json = Json { ignoreUnknownKeys = true }

    private val fetchShim: String by lazy { readAsset("plugins/polyfills.js") }
    private val domShim: String by lazy { readAsset("plugins/polyfills_dom.js") }
    private val cryptoShim: String by lazy { readAsset("plugins/polyfills_crypto.js") }

    fun execute(provider: PluginProvider, script: String, request: ProviderRequest): PluginOutcome {
        if (script.length > MAX_SCRIPT_CHARS) {
            return PluginOutcome(error = "${provider.name}: the provider script is too large.")
        }
        return try {
            run(provider, script, request)
        } catch (error: Throwable) {
            PluginOutcome(error = "${provider.name}: ${error.message ?: "the provider failed."}")
        }
    }

    private fun run(provider: PluginProvider, script: String, request: ProviderRequest): PluginOutcome {
        val deadline = System.currentTimeMillis() + DEADLINE_MS
        val logs = mutableListOf<String>()
        var capturedResult: String? = null
        var capturedError: String? = null
        val dom = PluginDom()
        val argsJson = json.encodeToString(
            CallArgs(
                tmdbId = request.tmdbId,
                mediaType = request.mediaType,
                season = request.season,
                episode = request.episode,
            ),
        )
        val settings = provider.settings.takeIf { it.isNotBlank() } ?: "{}"
        try {
            QuickJSLoader.init()
            val runtime = QuickJSContext.create()
            this.runtime = runtime
            runtime.setMemoryLimit(MEMORY_LIMIT)
            runtime.setMaxStackSize(STACK_LIMIT)
            val global = runtime.getGlobalObject()

            global.setProperty("__senzo_scraper_id", function { _ -> provider.key })
            global.setProperty("__senzo_settings_json", function { _ -> settings })
            global.setProperty("__senzo_call_args", function { _ -> argsJson })
            global.setProperty("__senzo_capture_result", function { args ->
                capturedResult = args.firstOrNull()?.toString()
                null
            })
            global.setProperty("__senzo_capture_error", function { args ->
                capturedError = args.firstOrNull()?.toString()
                null
            })
            global.setProperty("__senzo_log", function { args ->
                val level = args.getOrNull(0)?.toString().orEmpty()
                val message = args.getOrNull(1)?.toString().orEmpty()
                if (logs.size < MAX_LOG_LINES) {
                    logs.add(level + ": " + message.take(800))
                }
                null
            })
            global.setProperty("__senzo_sleep", function { args ->
                checkDeadline(deadline)
                val millis = (args.firstOrNull() as? Number)?.toLong() ?: 0L
                Thread.sleep(millis.coerceIn(0, 60_000))
                null
            })
            global.setProperty("__senzo_fetch", function { args ->
                checkDeadline(deadline)
                val raw = args.firstOrNull()?.toString()
                    ?: throw IllegalArgumentException("The provider sent an invalid fetch request.")
                val requestBody = json.decodeFromString<FetchRequest>(raw)
                json.encodeToString(fetch.fetch(requestBody))
            })
            global.setProperty("__senzo_parse_url", function { args ->
                dom.parseUrl(args.firstOrNull()?.toString().orEmpty())
            })
            global.setProperty("__senzo_text_encode", function { args ->
                args.firstOrNull()?.toString().orEmpty().toByteArray(Charsets.UTF_8).toHex()
            })
            global.setProperty("__senzo_text_decode", function { args ->
                val bytes = decodeHex(args.firstOrNull()?.toString().orEmpty())
                String(bytes, Charsets.UTF_8)
            })

            global.setProperty("__crypto_digest_hex", function { args ->
                PluginCrypto.digest(args.string(0), args.string(1))
            })
            global.setProperty("__crypto_hmac_hex", function { args ->
                PluginCrypto.hmac(args.string(0), args.string(1), args.string(2))
            })
            global.setProperty("__crypto_pbkdf2_hex", function { args ->
                PluginCrypto.pbkdf2(
                    password = args.string(0),
                    salt = args.string(1),
                    iterations = (args.getOrNull(2) as? Number)?.toInt() ?: 0,
                    bits = (args.getOrNull(3) as? Number)?.toInt() ?: 0,
                    hash = args.string(4),
                )
            })
            global.setProperty("__crypto_aes_encrypt_hex", function { args ->
                PluginCrypto.aes(true, args.string(0), args.string(1), args.string(2), args.string(3))
            })
            global.setProperty("__crypto_aes_decrypt_hex", function { args ->
                PluginCrypto.aes(false, args.string(0), args.string(1), args.string(2), args.string(3))
            })
            global.setProperty("__crypto_random_hex", function { args ->
                PluginCrypto.randomHex((args.firstOrNull() as? Number)?.toInt() ?: 0)
            })

            global.setProperty("__cheerio_load", function { args ->
                dom.load(args.string(0))
            })
            global.setProperty("__cheerio_select", function { args ->
                dom.select(args.int(0), args.string(1))
            })
            global.setProperty("__cheerio_find", function { args ->
                dom.find(args.int(0), args.int(1), args.string(2))
            })
            global.setProperty("__cheerio_children", function { args ->
                dom.children(args.int(0), args.int(1), args.string(2))
            })
            global.setProperty("__cheerio_text", function { args ->
                dom.text(args.int(0), args.int(1))
            })
            global.setProperty("__cheerio_html", function { args ->
                dom.html(args.int(0), args.int(1))
            })
            global.setProperty("__cheerio_inner_html", function { args ->
                dom.innerHtml(args.int(0), args.int(1))
            })
            global.setProperty("__cheerio_attr", function { args ->
                dom.attr(args.int(0), args.int(1), args.string(2)) ?: "__UNDEFINED__"
            })
            global.setProperty("__cheerio_next", function { args ->
                dom.next(args.int(0), args.int(1))?.toString() ?: "__NONE__"
            })
            global.setProperty("__cheerio_prev", function { args ->
                dom.prev(args.int(0), args.int(1))?.toString() ?: "__NONE__"
            })
            global.setProperty("__cheerio_parent", function { args ->
                dom.parent(args.int(0), args.int(1))?.toString() ?: "__NONE__"
            })
            global.setProperty("__cheerio_is", function { args ->
                dom.`is`(args.int(0), args.int(1), args.string(2))
            })
            global.setProperty("__cheerio_closest", function { args ->
                dom.closest(args.int(0), args.int(1), args.string(2))?.toString() ?: "__NONE__"
            })

            runtime.evaluate(PRELUDE)
            runtime.evaluate(fetchShim)
            runtime.evaluate(domShim)
            runtime.evaluate(cryptoShim)
            runtime.evaluate(script)
            runtime.evaluate(CALL_CODE)
        } catch (error: Throwable) {
            capturedError = capturedError ?: (error.message ?: "the provider crashed.")
        } finally {
            runCatching { runtime?.destroy() }
            this.runtime = null
        }
        val raw = capturedResult
        if (raw == null) {
            val message = capturedError ?: "The provider returned nothing."
            return PluginOutcome(error = "${provider.name}: $message", logs = logs)
        }
        val streams = parseStreams(raw)
        return PluginOutcome(streams = streams, logs = logs)
    }

    @Volatile private var runtime: QuickJSContext? = null

    private fun checkDeadline(deadline: Long) {
        if (System.currentTimeMillis() > deadline) {
            throw IllegalStateException("The provider ran out of time.")
        }
    }

    private fun parseStreams(raw: String): List<PluginStream> {
        val value = runCatching { json.parseToJsonElement(raw) }.getOrNull() ?: return emptyList()
        val items: JsonArray = when (value) {
            is JsonArray -> value
            is JsonObject -> value["streams"]?.jsonArray ?: return emptyList()
            else -> return emptyList()
        }
        val streams = mutableListOf<PluginStream>()
        for (item in items.take(MAX_RESULT_STREAMS * 2)) {
            val objectValue = item as? JsonObject ?: continue
            val name = stringField(objectValue, "name", "provider", "source")
            val title = stringField(objectValue, "title", "description", "name")
            val magnet = firstMagnet(objectValue)
            var url = stringField(objectValue, "url", "externalUrl", "link") ?: magnet
            var infoHash = stringField(objectValue, "infoHash", "info_hash", "hash")
                ?: url?.let { magnetHash(it) }
            val validHash = infoHash?.let { isInfoHash(it) } == true
            if (!validHash) infoHash = null
            val playable = url?.let {
                it.startsWith("http://") || it.startsWith("https://") ||
                    (it.startsWith("magnet:") && validHash)
            } == true
            if (!playable && !validHash) continue
            val fileIdx = objectValue.intField("fileIdx", "file_idx")
            val headers = headerMap(objectValue)
            val quality = stringField(objectValue, "quality", "resolution")
            val displayName = when {
                quality != null && name != null && !name.contains(quality) -> "$name · $quality"
                name != null -> name
                quality != null -> quality
                else -> "Plugin source"
            }
            val description = title ?: displayName
            val seeders = objectValue.numberField("seeders", "seeds", "peers")
                ?: seedersFromText(description)
                ?: seedersFromText(displayName)
            if (streams.any { existing ->
                    (existing.url != null && existing.url == url) ||
                        (existing.infoHash != null && existing.infoHash == infoHash)
                }
            ) {
                continue
            }
            streams.add(
                PluginStream(
                    name = displayName.take(160),
                    description = description.take(500),
                    url = url?.takeIf { it.startsWith("http") || it.startsWith("magnet:") },
                    infoHash = infoHash,
                    fileIdx = fileIdx,
                    headers = headers,
                    seeders = seeders,
                ),
            )
            if (streams.size >= MAX_RESULT_STREAMS) break
        }
        return streams
    }

    private fun firstMagnet(value: JsonObject): String? {
        (value["magnet"] as? JsonPrimitive)?.contentOrNull
            ?.takeIf { it.startsWith("magnet:") }
            ?.let { return it }
        val sources = value["sources"] as? JsonArray ?: return null
        return sources.asSequence()
            .mapNotNull { (it as? JsonPrimitive)?.contentOrNull }
            .firstOrNull { it.startsWith("magnet:") }
    }

    private fun headerMap(value: JsonObject): Map<String, String> {
        val hints = value["behaviorHints"] as? JsonObject
        val proxy = hints?.get("proxyHeaders") as? JsonObject
        val request = proxy?.get("request") as? JsonObject
            ?: value["headers"] as? JsonObject
            ?: return emptyMap()
        return request.entries.mapNotNull { (key, entry) ->
            (entry as? JsonPrimitive)?.contentOrNull?.let { key to it }
        }.toMap()
    }

    private fun stringField(value: JsonObject, vararg keys: String): String? {
        for (key in keys) {
            val text = (value[key] as? JsonPrimitive)?.contentOrNull?.trim()
            if (!text.isNullOrEmpty()) return text
        }
        return null
    }

    private fun JsonObject.intField(vararg keys: String): Int? {
        for (key in keys) {
            val primitive = this[key] as? JsonPrimitive ?: continue
            return primitive.intOrNull ?: primitive.contentOrNull?.toIntOrNull()
        }
        return null
    }

    private fun JsonObject.numberField(vararg keys: String): Int? {
        for (key in keys) {
            val primitive = this[key] as? JsonPrimitive ?: continue
            val number = primitive.intOrNull ?: primitive.contentOrNull?.trim()?.toIntOrNull()
            if (number != null) return number
        }
        return null
    }

    private fun seedersFromText(value: String): Int? {
        for (marker in SEED_MARKERS) {
            var search = 0
            while (true) {
                val relative = value.indexOf(marker, search)
                if (relative < 0) break
                val tail = value.substring(relative + marker.length)
                    .dropWhile { it.isWhitespace() || it == ':' || it == '=' }
                val digits = tail.takeWhile { it.isDigit() }
                digits.toIntOrNull()?.let { return it }
                search = relative + marker.length
            }
        }
        return null
    }

    private fun isInfoHash(value: String): Boolean {
        val clean = value.trim()
        if (clean.length == 40) return clean.all { it.isDigit() || it in 'a'..'f' || it in 'A'..'F' }
        if (clean.length == 32) {
            return clean.all { character ->
                character.isLetterOrDigit() || character == '='
            }
        }
        return false
    }

    private fun magnetHash(value: String): String? {
        if (!value.startsWith("magnet:")) return null
        val marker = value.indexOf("urn:btih:")
        if (marker < 0) return null
        val rest = value.substring(marker + "urn:btih:".length)
        val hash = rest.takeWhile { it != '&' }
        return hash.takeIf { isInfoHash(it) }
    }

    private fun readAsset(name: String): String {
        return context.assets.open(name).use { stream ->
            stream.readBytes().toString(Charsets.UTF_8)
        }
    }

    private fun function(body: (Array<out Any?>) -> Any?): JSCallFunction =
        object : JSCallFunction {
            override fun call(vararg args: Any?): Any? = body(args)
        }

    private fun Array<out Any?>.string(index: Int): String =
        getOrNull(index)?.toString().orEmpty()

    private fun Array<out Any?>.int(index: Int): Int =
        (getOrNull(index) as? Number)?.toInt() ?: 0

    private fun ByteArray.toHex(): String {
        val digits = "0123456789abcdef"
        val builder = StringBuilder(size * 2)
        for (byte in this) {
            val value = byte.toInt() and 0xFF
            builder.append(digits[value ushr 4]).append(digits[value and 0x0F])
        }
        return builder.toString()
    }

    private fun decodeHex(value: String): ByteArray {
        if (value.isEmpty() || value.length % 2 != 0) return ByteArray(0)
        return ByteArray(value.length / 2) { index ->
            val high = Character.digit(value[index * 2], 16)
            val low = Character.digit(value[index * 2 + 1], 16)
            if (high < 0 || low < 0) 0 else ((high shl 4) or low).toByte()
        }
    }

    private companion object {
        const val MEMORY_LIMIT = 128 * 1024 * 1024
        const val STACK_LIMIT = 2 * 1024 * 1024
        const val DEADLINE_MS = 60_000L
        const val MAX_LOG_LINES = 80
        const val MAX_RESULT_STREAMS = 48
        const val MAX_SCRIPT_CHARS = 4 * 1024 * 1024
        val SEED_MARKERS = listOf("👤", "Seeders:", "Seeders", "Seeds:", "Seeds")
        const val PRELUDE = "var module = { exports: {} };\nvar exports = module.exports;\n"
        const val CALL_CODE = """
(async function () {
  try {
    var getStreams = (typeof module !== 'undefined' && module.exports && module.exports.getStreams)
      || globalThis.getStreams;
    if (typeof getStreams !== 'function') {
      throw new Error('getStreams is not exported by this provider');
    }
    var args = JSON.parse(__senzo_call_args());
    var season = args.season == null ? undefined : args.season;
    var episode = args.episode == null ? undefined : args.episode;
    var result = await getStreams(args.tmdbId, args.mediaType, season, episode);
    __senzo_capture_result(JSON.stringify(result || []));
  } catch (error) {
    __senzo_capture_error(error && error.message ? error.message : String(error));
  }
})();
"""
    }
}
