package com.senzo.app

import android.content.Context
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString

/**
 * Everything the app remembers between launches: the saved library, the
 * resume points of the board, recent searches, addon state and preferences.
 *
 * Preferences are private to the app and small, so a single file is enough;
 * the desktop client writes the same kind of state to its own store.
 */
class PrefsStore(context: Context) {
    private val prefs = context.getSharedPreferences("senzo.state", Context.MODE_PRIVATE)

    fun library(): List<LibraryEntry> = decode(KEY_LIBRARY) ?: emptyList()

    fun saveLibrary(entries: List<LibraryEntry>) {
        encode(KEY_LIBRARY, entries)
    }

    fun progress(): List<WatchProgress> = decode(KEY_PROGRESS) ?: emptyList()

    fun progressFor(key: String): WatchProgress? = progress().firstOrNull { it.key == key }

    /** Stores a resume point, replacing the older one for the same video. */
    fun saveProgress(entry: WatchProgress) {
        val existing = progress()
            .filterNot { it.key == entry.key }
            .plus(entry.copy(updatedAt = System.currentTimeMillis()))
            .sortedByDescending { it.updatedAt }
            .take(MAX_PROGRESS)
        encode(KEY_PROGRESS, existing)
    }

    fun removeProgress(key: String) {
        encode(KEY_PROGRESS, progress().filterNot { it.key == key })
    }

    fun clearProgress() {
        prefs.edit().remove(KEY_PROGRESS).apply()
    }

    fun searches(): List<String> = decode(KEY_SEARCHES) ?: emptyList()

    fun addSearch(query: String) {
        val trimmed = query.trim()
        if (trimmed.isEmpty()) return
        val next = (listOf(trimmed) + searches().filterNot { it.equals(trimmed, ignoreCase = true) })
            .take(MAX_SEARCHES)
        encode(KEY_SEARCHES, next)
    }

    fun clearSearches() {
        prefs.edit().remove(KEY_SEARCHES).apply()
    }

    fun settings(): AppSettings = decode(KEY_SETTINGS) ?: AppSettings()

    fun saveSettings(settings: AppSettings) {
        encode(KEY_SETTINGS, settings)
    }

    fun disabledAddons(): Set<String> = (decode<List<String>>(KEY_DISABLED) ?: emptyList()).toSet()

    fun saveDisabledAddons(ids: Set<String>) {
        encode(KEY_DISABLED, ids.toList())
    }

    fun addonOrder(): List<String> = decode(KEY_ORDER) ?: emptyList()

    fun saveAddonOrder(order: List<String>) {
        encode(KEY_ORDER, order)
    }

    /** Addons installed on this device, which the account collection lacks. */
    fun localAddons(): List<AddonDescriptor> = decode(KEY_LOCAL) ?: emptyList()

    fun saveLocalAddons(addons: List<AddonDescriptor>) {
        encode(KEY_LOCAL, addons)
    }

    /**
     * The local profile: no account, the official addons, and everything the
     * viewer saves staying on this device. The desktop client calls this guest
     * mode and remembers the choice across restarts; so does this.
     */
    fun guest(): Boolean = prefs.getBoolean(KEY_GUEST, false)

    fun saveGuest(enabled: Boolean) {
        prefs.edit().putBoolean(KEY_GUEST, enabled).apply()
    }

    /** The official addons of the guest profile, cached so it starts offline. */
    fun guestAddons(): List<AddonDescriptor> = decode(KEY_GUEST_ADDONS) ?: emptyList()

    fun saveGuestAddons(addons: List<AddonDescriptor>) {
        encode(KEY_GUEST_ADDONS, addons)
    }

    private inline fun <reified T> decode(key: String): T? {
        val raw = prefs.getString(key, null) ?: return null
        return runCatching { Http.json.decodeFromString<T>(raw) }.getOrNull()
    }

    private inline fun <reified T> encode(key: String, value: T) {
        val raw = runCatching { Http.json.encodeToString(value) }.getOrNull() ?: return
        prefs.edit().putString(key, raw).apply()
    }

    private companion object {
        const val KEY_LIBRARY = "library"
        const val KEY_PROGRESS = "progress"
        const val KEY_SEARCHES = "searches"
        const val KEY_SETTINGS = "settings"
        const val KEY_DISABLED = "disabledAddons"
        const val KEY_ORDER = "addonOrder"
        const val KEY_LOCAL = "localAddons"
        const val KEY_GUEST = "guestMode"
        const val KEY_GUEST_ADDONS = "guestAddons"
        const val MAX_PROGRESS = 80
        const val MAX_SEARCHES = 8
    }
}