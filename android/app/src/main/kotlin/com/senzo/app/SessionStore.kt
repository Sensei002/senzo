package com.senzo.app

import android.content.Context
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString

/**
 * Sign-in state and the last addon collection, kept in private preferences.
 *
 * The collection is cached so the board paints on the next start without
 * waiting for the account API, which is the same trade the desktop client
 * makes with its own storage.
 */
class SessionStore(context: Context) {
    private val preferences =
        context.getSharedPreferences("senzo.session", Context.MODE_PRIVATE)

    fun session(): Session? {
        val authKey = preferences.getString(KEY_AUTH, null)?.takeIf { it.isNotBlank() }
            ?: return null
        val email = preferences.getString(KEY_EMAIL, "").orEmpty()
        return Session(authKey = authKey, email = email)
    }

    fun saveSession(session: Session) {
        preferences.edit()
            .putString(KEY_AUTH, session.authKey)
            .putString(KEY_EMAIL, session.email)
            .apply()
    }

    fun clearSession() {
        preferences.edit()
            .remove(KEY_AUTH)
            .remove(KEY_EMAIL)
            .remove(KEY_ADDONS)
            .apply()
    }

    fun addons(): List<AddonDescriptor> {
        val cached = preferences.getString(KEY_ADDONS, null) ?: return emptyList()
        return runCatching {
            Http.json.decodeFromString<List<AddonDescriptor>>(cached)
        }.getOrDefault(emptyList())
    }

    fun saveAddons(addons: List<AddonDescriptor>) {
        val encoded = runCatching { Http.json.encodeToString(addons) }.getOrNull() ?: return
        preferences.edit().putString(KEY_ADDONS, encoded).apply()
    }

    private companion object {
        const val KEY_AUTH = "authKey"
        const val KEY_EMAIL = "email"
        const val KEY_ADDONS = "addons"
    }
}
