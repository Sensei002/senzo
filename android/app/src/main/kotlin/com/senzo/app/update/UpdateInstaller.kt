package com.senzo.app.update

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import okhttp3.OkHttpClient
import okhttp3.Request
import java.io.File
import java.util.concurrent.TimeUnit

/** Downloads a release APK and hands it to the system package installer. */
object UpdateInstaller {
    private const val APK_MIME = "application/vnd.android.package-archive"

    fun canInstall(context: Context): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.O ||
            context.packageManager.canRequestPackageInstalls()

    /** Sends the viewer to the per-app "install unknown apps" switch. */
    fun requestInstallPermission(context: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val intent = Intent(
            Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
            Uri.parse("package:" + context.packageName),
        ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        runCatching { context.startActivity(intent) }
    }

    fun openPage(context: Context, pageUrl: String) {
        val intent = Intent(Intent.ACTION_VIEW, Uri.parse(pageUrl))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        runCatching { context.startActivity(intent) }
    }

    fun download(context: Context, info: UpdateInfo, onProgress: (Int) -> Unit): File {
        val url = info.apkUrl ?: error("This release does not publish an APK.")
        val directory = File(context.cacheDir, "updates").apply { mkdirs() }
        val target = File(directory, "senzo-" + info.version + ".apk")
        val client = OkHttpClient.Builder()
            .connectTimeout(20, TimeUnit.SECONDS)
            .readTimeout(60, TimeUnit.SECONDS)
            .build()
        val request = Request.Builder()
            .url(url)
            .header("User-Agent", "SENZO-Android")
            .build()
        client.newCall(request).execute().use { response ->
            if (!response.isSuccessful) error("Download failed (" + response.code + ").")
            val body = response.body ?: error("The release APK was empty.")
            val total = body.contentLength()
            body.byteStream().use { input ->
                target.outputStream().use { output ->
                    val buffer = ByteArray(64 * 1024)
                    var written = 0L
                    while (true) {
                        val read = input.read(buffer)
                        if (read < 0) break
                        output.write(buffer, 0, read)
                        written += read
                        if (total > 0) {
                            onProgress(((written * 100) / total).toInt().coerceIn(0, 100))
                        }
                    }
                }
            }
        }
        return target
    }

    fun install(context: Context, file: File): Boolean {
        val uri = FileProvider.getUriForFile(
            context,
            context.packageName + ".updates",
            file,
        )
        val intent = Intent(Intent.ACTION_VIEW)
            .setDataAndType(uri, APK_MIME)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        return runCatching {
            context.startActivity(intent)
            true
        }.getOrDefault(false)
    }
}