package com.senzo.app.plugins

import org.jsoup.Jsoup
import org.jsoup.nodes.Document
import org.jsoup.nodes.Element
import java.util.IdentityHashMap
import java.util.Locale

/**
 * The HTML side of a provider script.
 *
 * Providers are written against a small jQuery/Cheerio subset; the desktop
 * runtime exposes that subset over a native tree. jsoup gives the same shape on
 * Android, so the shipped JS facade runs unchanged: every element is addressed
 * by a stable integer id, and documents are numbered from one.
 */
class PluginDom {

    private class Stored(val document: Document) {
        val nodes: List<Element> = document.getAllElements()
        val index = IdentityHashMap<Element, Int>().apply {
            nodes.forEachIndexed { position, element -> put(element, position) }
        }
    }

    private val documents = mutableListOf<Stored>()

    @Synchronized
    fun load(html: String): Int {
        documents.add(Stored(Jsoup.parse(html)))
        return documents.size
    }

    @Synchronized
    fun select(documentId: Int, selector: String): String {
        val stored = document(documentId) ?: return "[]"
        val elements = runCatching { stored.document.select(selector) }.getOrNull()
            ?: return "[]"
        return encode(stored, elements)
    }

    @Synchronized
    fun find(documentId: Int, elementId: Int, selector: String): String {
        val stored = document(documentId) ?: return "[]"
        val element = element(stored, elementId) ?: return "[]"
        val elements = runCatching { element.select(selector) }.getOrNull() ?: return "[]"
        return encode(stored, elements)
    }

    @Synchronized
    fun children(documentId: Int, elementId: Int, selector: String): String {
        val stored = document(documentId) ?: return "[]"
        val element = element(stored, elementId) ?: return "[]"
        val children = element.children().filter { child ->
            runCatching { child.`is`(selector.ifBlank { "*" }) }.getOrDefault(false)
        }
        return encode(stored, children)
    }

    @Synchronized
    fun text(documentId: Int, elementId: Int): String {
        val stored = document(documentId) ?: return ""
        return element(stored, elementId)?.text().orEmpty()
    }

    @Synchronized
    fun innerHtml(documentId: Int, elementId: Int): String {
        val stored = document(documentId) ?: return ""
        return element(stored, elementId)?.html().orEmpty()
    }

    @Synchronized
    fun html(documentId: Int, elementId: Int): String {
        val stored = document(documentId) ?: return ""
        if (elementId == 0) return stored.document.html()
        return element(stored, elementId)?.outerHtml().orEmpty()
    }

    @Synchronized
    fun attr(documentId: Int, elementId: Int, name: String): String? {
        val value = element(document(documentId) ?: return null, elementId)
            ?.takeIf { it.hasAttr(name) }
            ?.attr(name)
        return value
    }

    @Synchronized
    fun next(documentId: Int, elementId: Int): Int? {
        val stored = document(documentId) ?: return null
        val sibling = element(stored, elementId)?.nextElementSibling() ?: return null
        return stored.index[sibling]
    }

    @Synchronized
    fun prev(documentId: Int, elementId: Int): Int? {
        val stored = document(documentId) ?: return null
        val sibling = element(stored, elementId)?.previousElementSibling() ?: return null
        return stored.index[sibling]
    }

    @Synchronized
    fun parent(documentId: Int, elementId: Int): Int? {
        val stored = document(documentId) ?: return null
        val parent = element(stored, elementId)?.parent() ?: return null
        return stored.index[parent]
    }

    @Synchronized
    fun `is`(documentId: Int, elementId: Int, selector: String): Boolean {
        val element = element(document(documentId) ?: return false, elementId) ?: return false
        return runCatching { element.`is`(selector) }.getOrDefault(false)
    }

    @Synchronized
    fun closest(documentId: Int, elementId: Int, selector: String): Int? {
        val stored = document(documentId) ?: return null
        var current = element(stored, elementId) ?: return null
        while (true) {
            if (runCatching { current.`is`(selector) }.getOrDefault(false)) {
                return stored.index[current]
            }
            current = current.parent() ?: return null
        }
    }

    private fun document(id: Int): Stored? = documents.getOrNull(id - 1)

    private fun element(stored: Stored, id: Int): Element? = stored.nodes.getOrNull(id)

    private fun encode(stored: Stored, elements: Collection<Element>): String {
        val ids = elements.mapNotNull { stored.index[it] }
        return ids.joinToString(",", prefix = "[", postfix = "]")
    }

    /** The URL view provider scripts expect from `new URL(...)`. */
    fun parseUrl(value: String): String {
        val url = runCatching { java.net.URI(value.trim()) }.getOrNull()
            ?: return "{\"error\":\"invalid url\"}"
        val scheme = url.scheme?.lowercase(Locale.US) ?: return "{\"error\":\"invalid url\"}"
        val host = url.host.orEmpty()
        val hostWithPort = if (url.port > 0) {
            host + ":" + url.port
        } else {
            host
        }
        val query = url.rawQuery?.let { "?$it" } ?: ""
        val fragment = url.rawFragment?.let { "#$it" } ?: ""
        return buildString {
            append("{\"href\":").append(quote(value))
            append(",\"protocol\":").append(quote("$scheme:"))
            append(",\"username\":").append(quote(url.userInfo?.substringBefore(":") ?: ""))
            append(",\"password\":").append(quote(url.userInfo?.substringAfter(":", "") ?: ""))
            append(",\"host\":").append(quote(hostWithPort))
            append(",\"hostname\":").append(quote(host))
            append(",\"port\":").append(quote(if (url.port > 0) url.port.toString() else ""))
            append(",\"pathname\":").append(quote(url.rawPath.orEmpty()))
            append(",\"search\":").append(quote(query))
            append(",\"hash\":").append(quote(fragment))
            append(",\"origin\":").append(quote("$scheme://$host"))
            append("}")
        }
    }

    private fun quote(value: String): String = buildString {
        append('"')
        for (character in value) {
            when (character) {
                '"' -> append("\\\"")
                '\\' -> append("\\\\")
                '\n' -> append("\\n")
                '\r' -> append("\\r")
                '\t' -> append("\\t")
                else -> append(character)
            }
        }
        append('"')
    }
}
