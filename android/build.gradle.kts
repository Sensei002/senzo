// The Android client mirrors the desktop application: the same Stremio addon
// protocol, the same catalogue and stream layout, rendered with Jetpack Compose
// instead of Slint.
plugins {
    id("com.android.application") version "8.7.3" apply false
    // Kotlin 2.1 also reads the FFmpeg decoder library, which is built with it.
    id("org.jetbrains.kotlin.android") version "2.1.0" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.1.0" apply false
    id("org.jetbrains.kotlin.plugin.serialization") version "2.1.0" apply false
}
