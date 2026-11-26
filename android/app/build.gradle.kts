import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
}

// Version and signing arrive from CI so one workflow publishes the desktop and
// Android builds of the same tag. A local build falls back to the desktop
// version and an unsigned release.
val desktopManifest = rootProject.file("../app/Cargo.toml")
val desktopVersion: String = if (desktopManifest.isFile) {
    desktopManifest.readLines()
        .firstOrNull { line -> line.startsWith("version") && line.contains('"') }
        ?.substringAfter('"')
        ?.substringBefore('"')
        ?.takeIf { it.isNotBlank() }
        ?: "0.0.0"
} else {
    "0.0.0"
}

fun versionCodeOf(version: String): Int {
    val parts = version.split("-", "+", ".").mapNotNull { it.toIntOrNull() }
    val major = parts.getOrElse(0) { 0 }
    val minor = parts.getOrElse(1) { 0 }
    val patch = parts.getOrElse(2) { 0 }
    return major * 10_000 + minor * 100 + patch
}

val appVersionName: String = (findProperty("senzoVersionName") as String?) ?: desktopVersion
val appVersionCode: Int = (findProperty("senzoVersionCode") as String?)?.toIntOrNull()
    ?: versionCodeOf(appVersionName)

val keystoreProperties = Properties().apply {
    val file = file("keystore.properties")
    if (file.isFile) file.inputStream().use { load(it) }
}

fun signingValue(key: String, environment: String): String? =
    (keystoreProperties.getProperty(key) ?: System.getenv(environment))?.takeIf { it.isNotBlank() }

val releaseKeystore = signingValue("storeFile", "SENZO_KEYSTORE")
val releaseStorePassword = signingValue("storePassword", "SENZO_KEYSTORE_PASSWORD")
val releaseKeyAlias = signingValue("keyAlias", "SENZO_KEY_ALIAS")
val releaseKeyPassword = signingValue("keyPassword", "SENZO_KEY_PASSWORD")
// Absent means the JVM default store type, which is PKCS#12 on modern JDKs.
val releaseStoreType = signingValue("storeType", "SENZO_KEYSTORE_TYPE")

android {
    namespace = "com.senzo.app"
    compileSdk = 35

    // Signing is optional: without a keystore the release build stays unsigned,
    // so a local build never needs a private key.
    val releaseSigning = releaseKeystore?.let { storePath ->
        signingConfigs.create("release") {
            storeFile = file(storePath)
            storePassword = releaseStorePassword
            keyAlias = releaseKeyAlias
            keyPassword = releaseKeyPassword
            releaseStoreType?.let { storeType = it }
        }
    }

    defaultConfig {
        applicationId = "com.senzo.app"
        minSdk = 26
        targetSdk = 35
        versionCode = appVersionCode
        versionName = appVersionName

        // The torrent engine ships a native build per architecture. 32-bit x86
        // has no phone behind it, so the APK carries the two that matter and
        // the emulator build is only produced when it is asked for.
        ndk {
            abiFilters += listOf("arm64-v8a", "armeabi-v7a")
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    // libtorrent is ~15 MB per architecture, so a universal APK would be three
    // copies of the same engine. One APK per ABI keeps the download around
    // 18 MB; phones take the arm64 build.
    splits {
        abi {
            isEnable = true
            reset()
            include("arm64-v8a", "armeabi-v7a")
            isUniversalApk = false
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            if (releaseSigning != null) {
                signingConfig = releaseSigning
            }
        }
        debug {
            applicationIdSuffix = ".debug"
            versionNameSuffix = "-debug"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    packaging {
        resources.excludes += setOf("/META-INF/{AL2.0,LGPL2.1}")
    }

    lint {
        // The release build is a CI artifact; a lint warning must not block it.
        abortOnError = false
        checkReleaseBuilds = false
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2024.10.01")
    implementation(composeBom)
    androidTestImplementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")

    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")

    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("io.coil-kt:coil-compose:2.7.0")

    implementation("androidx.media3:media3-exoplayer:1.5.0")
    implementation("androidx.media3:media3-ui:1.5.0")
    implementation("androidx.media3:media3-datasource-okhttp:1.5.0")

    // Provider plugins: Nuvio scripts run on QuickJS, with jsoup as their HTML tree.
    implementation("wang.harlon.quickjs:wrapper-android:3.2.3")
    implementation("org.jsoup:jsoup:1.18.3")

    // BitTorrent, for the infohash streams Torrentio and the plugins return.
    implementation("org.libtorrent4j:libtorrent4j:2.1.0-39")
    implementation("org.libtorrent4j:libtorrent4j-android-arm64:2.1.0-39")
    implementation("org.libtorrent4j:libtorrent4j-android-arm:2.1.0-39")
    implementation("org.libtorrent4j:libtorrent4j-android-x86_64:2.1.0-39")

    debugImplementation("androidx.compose.ui:ui-tooling")
    testImplementation("junit:junit:4.13.2")
}
