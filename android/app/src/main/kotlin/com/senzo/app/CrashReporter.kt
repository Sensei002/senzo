package com.senzo.app

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.os.Build
import android.util.TypedValue
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import java.io.File
import java.io.PrintWriter
import java.io.StringWriter

/**
 * Turns a crash into something the person holding the phone can act on.
 *
 * A release build of this app is minified, so the only way to pin down a
 * failure on a device that is not attached to a debugger is to carry the
 * trace out of the app itself. The last crash is written to private storage
 * and shown on the next launch, where it can be copied into a message.
 */
object CrashReporter {
    private const val FILE_NAME = "last-crash.txt"

    /** Catches uncaught failures on any thread, keeps the trace, then rethrows. */
    fun install(context: Context) {
        val app = context.applicationContext
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, error ->
            runCatching { record(app, error) }
            previous?.uncaughtException(thread, error)
        }
    }

    /** Remembers a failure that was caught before it could kill the process. */
    fun record(context: Context, error: Throwable): String {
        val trace = StringWriter()
        PrintWriter(trace).use { error.printStackTrace(it) }
        val details = buildString {
            appendLine("SENZO crash")
            appendLine("version " + BuildConfig.VERSION_NAME + " (" + BuildConfig.VERSION_CODE + ")")
            appendLine(
                "device " + Build.MANUFACTURER + " " + Build.MODEL +
                    ", Android " + Build.VERSION.RELEASE + " (API " + Build.VERSION.SDK_INT + ")",
            )
            appendLine()
            append(trace)
        }
        runCatching { File(context.filesDir, FILE_NAME).writeText(details) }
        return details
    }

    /** The trace kept by the previous run, if that run ended in a crash. */
    fun pending(context: Context): String? {
        val file = File(context.filesDir, FILE_NAME)
        if (!file.isFile) return null
        return runCatching { file.readText() }.getOrNull()?.takeIf { it.isNotBlank() }
    }

    fun forget(context: Context) {
        runCatching { File(context.filesDir, FILE_NAME).delete() }
    }

    /** A readable crash page, built without Compose so it works when Compose did not. */
    fun page(context: Context, details: String, onContinue: () -> Unit): View {
        fun dp(value: Int) = (value * context.resources.displayMetrics.density).toInt()
        val root = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.parseColor("#08090C"))
            setPadding(dp(20), dp(56), dp(20), dp(20))
        }
        root.addView(
            TextView(context).apply {
                text = "SENZO stopped unexpectedly"
                setTextColor(Color.parseColor("#F4F6F8"))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 20f)
                setTypeface(typeface, Typeface.BOLD)
            },
        )
        root.addView(
            TextView(context).apply {
                text = "The report below is what fixes this. Copy it and send it on."
                setTextColor(Color.parseColor("#9BA6B2"))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
                setPadding(0, dp(8), 0, dp(16))
            },
        )
        val scroller = ScrollView(context)
        scroller.addView(
            TextView(context).apply {
                text = details
                setTextColor(Color.parseColor("#C7D0D9"))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 11f)
                typeface = Typeface.MONOSPACE
                setTextIsSelectable(true)
            },
        )
        root.addView(scroller, LinearLayout.LayoutParams(-1, 0, 1f))

        val row = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(0, dp(16), 0, 0)
        }
        row.addView(
            Button(context).apply {
                text = "Copy details"
                setOnClickListener {
                    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                    clipboard.setPrimaryClip(ClipData.newPlainText("SENZO crash", details))
                    Toast.makeText(context, "Crash details copied", Toast.LENGTH_SHORT).show()
                }
            },
            LinearLayout.LayoutParams(0, -2, 1f),
        )
        row.addView(
            Button(context).apply {
                text = "Continue"
                setOnClickListener { onContinue() }
            },
            LinearLayout.LayoutParams(0, -2, 1f),
        )
        root.addView(row)
        return root
    }
}
