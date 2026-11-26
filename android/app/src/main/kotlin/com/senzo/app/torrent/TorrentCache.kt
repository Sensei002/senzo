package com.senzo.app.torrent

import java.io.File

/**
 * What happens to the pieces the engine wrote for a previous run.
 *
 * The desktop client rotates its torrent cache at launch: a new engine cannot
 * resume what an old one wrote, so the directory is renamed out of the way in
 * one metadata call and a worker deletes it afterwards, which keeps a
 * multi-gigabyte delete off the startup path. The same rule applies here, and a
 * finished torrent's payload is dropped as soon as the engine releases it.
 */
object TorrentCache {
    private const val STALE_PREFIX = "torrents.stale-"

    /** Rotates [root] and deletes what it held, off the caller's thread. */
    fun rotate(root: File) {
        val parent = root.parentFile ?: return
        parent.mkdirs()
        val stale = ArrayList<File>()
        parent.listFiles()?.forEach { entry ->
            if (entry.isDirectory && entry.name.startsWith(STALE_PREFIX)) stale += entry
        }
        // A stream that is still open keeps its directory rather than failing
        // the launch; the next launch rotates it.
        if (root.isDirectory) {
            val moved = File(parent, STALE_PREFIX + System.currentTimeMillis())
            if (root.renameTo(moved)) stale += moved
        }
        root.mkdirs()
        removeInBackground(stale)
    }

    /** Deletes one torrent's payload once the engine let go of it. */
    fun discard(dir: File) {
        removeInBackground(listOf(dir))
    }

    private fun removeInBackground(paths: List<File>) {
        if (paths.isEmpty()) return
        val work = ArrayList(paths)
        Thread({
            work.forEach { path -> runCatching { path.deleteRecursively() } }
        }, "senzo-torrent-cache-cleanup").apply { isDaemon = true }.start()
    }
}
