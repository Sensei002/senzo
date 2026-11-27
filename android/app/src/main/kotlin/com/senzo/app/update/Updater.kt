package com.senzo.app.update

import android.os.Build
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.TimeUnit

/** A release that is newer than the build installed on the phone. */
data class UpdateInfo(
    val version: String,
    val notes: String,
    val pageUrl: String,
    val apkUrl: String?,
    val sizeBytes: Long,
)

/** What the settings page knows about the update check right now. */
sealed interface UpdateState {
    data object Idle : UpdateState
    data object Checking : UpdateState
    data class Available(val info: UpdateInfo) : UpdateState
    data object UpToDate : UpdateState
    data class Failed(val message: String) : UpdateState
}

/**
 * Reads the GitHub releases of the SENZO repository. A release only counts
 * when it publishes an APK, so the desktop-only tags of earlier versions do
 * not advertise an update the phone cannot install.
 */
object Updater {
    private const val RELEASES = "https://api.github.com/repos/Sensei002/senzo/releases?per_page=20"
    private const val RELEASES_PAGE = "https://github.com/Sensei002/senzo/releases"

    private val client = OkHttpClient.Builder()
        .connectTimeout(15, TimeUnit.SECONDS)
        .readTimeout(20, TimeUnit.SECONDS)
        .build()

    fun check(currentVersion: String): UpdateState {
        val request = Request.Builder()
            .url(RELEASES)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "SENZO-Android")
            .build()
        return try {
            client.newCall(request).execute().use { response ->
                if (!response.isSuccessful) {
                    return UpdateState.Failed("GitHub answered " + response.code + ".")
                }
                val releases = JSONArray(response.body?.string().orEmpty())
                val current = normalizeVersion(currentVersion)
                val candidate = (0 until releases.length())
                    .map { releases.getJSONObject(it) }
                    .filter { !it.optBoolean("draft") }
                    .mapNotNull { release ->
                        versionOf(release.optString("tag_name"))?.let { it to release }
                    }
                    .filter { (version, _) -> compareVersions(version, current) > 0 }
                    .maxWithOrNull { left, right -> compareVersions(left.first, right.first) }
                if (candidate == null) {
                    UpdateState.UpToDate
                } else {
                    UpdateState.Available(info(candidate.first, candidate.second))
                }
            }
        } catch (error: Exception) {
            UpdateState.Failed(error.message ?: "Could not reach GitHub.")
        }
    }

    private fun info(version: String, release: JSONObject): UpdateInfo {
        val assets = release.optJSONArray("assets") ?: JSONArray()
        val asset = apkAsset(assets)
        return UpdateInfo(
            version = version,
            notes = release.optString("body").trim(),
            pageUrl = release.optString("html_url").ifBlank { RELEASES_PAGE },
            apkUrl = asset?.optString("browser_download_url")?.takeIf { it.isNotBlank() },
            sizeBytes = asset?.optLong("size") ?: 0L,
        )
    }

    /** Picks the APK built for this phone, falling back to any phone build. */
    private fun apkAsset(assets: JSONArray): JSONObject? {
        val apks = (0 until assets.length())
            .map { assets.getJSONObject(it) }
            .filter { it.optString("name").endsWith(".apk", ignoreCase = true) }
        if (apks.isEmpty()) return null
        val abi = Build.SUPPORTED_ABIS.firstOrNull {
            it.startsWith("arm64") || it.startsWith("armeabi")
        }.orEmpty()
        val tokens = when {
            abi.startsWith("arm64") -> listOf("arm64")
            abi.startsWith("armeabi") -> listOf("arm32", "armeabi", "arm-v7")
            else -> emptyList()
        }
        val matching = apks.filter { asset ->
            val name = asset.optString("name").lowercase()
            tokens.any { name.contains(it) }
        }
        return matching.firstOrNull() ?: apks.firstOrNull()
    }

    /** Strips the tag prefix and any build suffix before comparing. */
    private fun normalizeVersion(value: String): String {
        val trimmed = value.trim().removePrefix("v")
        val end = trimmed.indexOfAny(charArrayOf('-', '+'))
        return if (end > 0) trimmed.substring(0, end) else trimmed
    }

    private fun versionOf(tag: String): String? {
        val version = normalizeVersion(tag)
        return version.takeIf { candidate ->
            candidate.isNotEmpty() && candidate.split('.').all { it.toIntOrNull() != null }
        }
    }

    private fun parts(version: String): List<Int> =
        version.split('.', '-', '+').mapNotNull { it.toIntOrNull() }

    private fun compareVersions(left: String, right: String): Int {
        val a = parts(left)
        val b = parts(right)
        for (index in 0 until maxOf(a.size, b.size)) {
            val value = a.getOrElse(index) { 0 }
            val other = b.getOrElse(index) { 0 }
            if (value != other) return value.compareTo(other)
        }
        return 0
    }
}