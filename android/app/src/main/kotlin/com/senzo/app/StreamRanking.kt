package com.senzo.app

/**
 * How the source page orders what the addons answered.
 *
 * Smart weighs the parts a viewer actually notices: resolution, source, codec,
 * HDR, audio, seeders and size. The other modes are the plain sorts the desktop
 * client offers, and Original keeps the order the addons replied in.
 */
enum class RankingMode(val label: String) {
    Smart("Smart"),
    Quality("Quality"),
    Smallest("Smallest"),
    Seeders("Seeders"),
    Original("Original"),
    ;

    companion object {
        fun of(value: String?): RankingMode =
            entries.firstOrNull { it.label.equals(value, ignoreCase = true) } ?: Smart
    }
}

/** One stream, its score, and why it scored the way it did. */
data class RankedStream(
    val item: StreamItem,
    /** Position in the addons' own reply, which Original keeps. */
    val index: Int,
    val score: Int,
    val filtered: Boolean,
    val qualityHeight: Int?,
    val sizeBytes: Long?,
    val seeders: Int?,
    val reasons: List<String>,
)

private data class ParsedStream(
    val qualityHeight: Int?,
    val source: String?,
    val codec: String?,
    val hdr: Boolean,
    val dolbyVision: Boolean,
    val audio: String?,
    val suspicious: Boolean,
)

/** Ranks and filters a list, keeping each stream's original position. */
fun rankStreams(
    streams: List<StreamItem>,
    mode: RankingMode,
    showFiltered: Boolean,
): List<RankedStream> {
    val ranked = streams.mapIndexed { index, item -> rankOne(index, item) }
        .filter { showFiltered || !it.filtered }
    return when (mode) {
        RankingMode.Original -> ranked.sortedBy { it.index }
        RankingMode.Quality -> ranked.sortedWith(
            compareByDescending<RankedStream> { it.qualityHeight ?: 0 }.thenBy { it.index },
        )
        RankingMode.Smallest -> ranked.sortedWith(
            compareBy<RankedStream> { it.sizeBytes ?: Long.MAX_VALUE }.thenBy { it.index },
        )
        RankingMode.Seeders -> ranked.sortedWith(
            compareByDescending<RankedStream> { it.seeders ?: 0 }.thenBy { it.index },
        )
        RankingMode.Smart -> ranked.sortedWith(
            compareByDescending<RankedStream> { it.score }
                .thenByDescending { it.qualityHeight ?: 0 }
                .thenBy { it.index },
        )
    }
}

private fun rankOne(index: Int, item: StreamItem): RankedStream {
    val parsed = parseStream(item)
    var score = 0
    val reasons = mutableListOf<String>()

    parsed.qualityHeight?.let { height ->
        val points = when {
            height >= 2160 -> 50
            height >= 1440 -> 42
            height >= 1080 -> 34
            height >= 720 -> 22
            height >= 480 -> 10
            else -> 2
        }
        score += points
        reasons += "${height}p video"
    }
    parsed.source?.let { source ->
        val points = when (source) {
            "BluRay", "REMUX" -> 18
            "WEB-DL" -> 13
            "WEBRip" -> 8
            "HDTV" -> 4
            "CAM", "TS" -> -35
            else -> 0
        }
        score += points
        reasons += "$source source"
    }
    parsed.codec?.let { codec ->
        score += when (codec) {
            "AV1" -> 10
            "HEVC" -> 8
            "H.264" -> 4
            else -> 0
        }
        reasons += "$codec codec"
    }
    if (parsed.dolbyVision) {
        score += 8
        reasons += "Dolby Vision"
    } else if (parsed.hdr) {
        score += 6
        reasons += "HDR video"
    }
    parsed.audio?.let { audio ->
        score += when {
            audio.contains("Atmos") || audio.contains("TrueHD") -> 8
            audio.contains("DTS") -> 6
            else -> 2
        }
        reasons += "$audio audio"
    }
    val seeders = item.seeds
    if (seeders != null) {
        score += when {
            seeders >= 500 -> 18
            seeders >= 100 -> 14
            seeders >= 25 -> 9
            seeders >= 5 -> 4
            seeders >= 1 -> 0
            else -> -18
        }
        reasons += "$seeders seeders"
    }
    val sizeBytes = item.sizeBytes ?: parseSizeBytes(item.detail) ?: parseSizeBytes(item.name)
    if (sizeBytes != null) {
        val gib = sizeBytes / 1_073_741_824.0
        score += when {
            gib <= 0.15 -> -10
            gib <= 8.0 -> 6
            gib >= 80.0 -> -12
            else -> 0
        }
        reasons += String.format(java.util.Locale.US, "%.1f GiB", gib)
    }
    if (parsed.suspicious) {
        score -= 1_000
        reasons += "Potential fake or scam result"
    }
    return RankedStream(
        item = item,
        index = index,
        score = score,
        filtered = parsed.suspicious,
        qualityHeight = parsed.qualityHeight,
        sizeBytes = sizeBytes,
        seeders = seeders,
        reasons = reasons,
    )
}

private fun parseStream(item: StreamItem): ParsedStream {
    val text = (item.name + " " + item.detail).uppercase()
    val qualityHeight = listOf(4320, 2160, 1440, 1080, 720, 576, 540, 480, 360)
        .firstOrNull { height ->
            text.contains("${height}P") ||
                (height == 2160 && text.contains("4K")) ||
                (height == 4320 && text.contains("8K"))
        }
    val source = listOf(
        "REMUX" to "REMUX",
        "BLURAY" to "BluRay",
        "BLU-RAY" to "BluRay",
        "WEB-DL" to "WEB-DL",
        "WEBDL" to "WEB-DL",
        "WEBRIP" to "WEBRip",
        "HDTV" to "HDTV",
        "CAM" to "CAM",
        "TELESYNC" to "TS",
    ).firstOrNull { (marker, _) -> text.contains(marker) }?.second
    val codec = listOf(
        listOf("AV1", "AV01") to "AV1",
        listOf("HEVC", "H265", "H.265", "X265") to "HEVC",
        listOf("H264", "H.264", "X264", "AVC") to "H.264",
    ).firstOrNull { (markers, _) -> markers.any { text.contains(it) } }?.second
    val dolbyVision = listOf("DOLBY VISION", "DOVI", " DV ").any { text.contains(it) }
    val hdr = dolbyVision || listOf("HDR10+", "HDR10", " HDR ", "HLG").any { text.contains(it) }
    val audio = listOf(
        "TRUEHD ATMOS",
        "ATMOS",
        "TRUEHD",
        "DTS-HD",
        "DTS",
        "EAC3",
        "DDP",
        "AAC",
    ).firstOrNull { text.contains(it) }
    val suspicious = listOf(
        "PASSWORD",
        "PASSW0RD",
        "CLICK HERE",
        "FREE BITCOIN",
        "INSTALL CODEC",
        "EXE INCLUDED",
        "SCAM",
        "FAKE",
    ).any { text.contains(it) }
    return ParsedStream(qualityHeight, source, codec, hdr, dolbyVision, audio, suspicious)
}

/** Bytes as the compact "12.4 GB" the source rows show. */
fun formatSize(bytes: Long): String {
    val gib = bytes / 1_073_741_824.0
    return if (gib >= 1.0) {
        String.format(java.util.Locale.US, "%.2f GB", gib)
    } else {
        String.format(java.util.Locale.US, "%.0f MB", bytes / 1_048_576.0)
    }
}

/** Reads the first "1.4 GB" style size in a line of addon text. */
fun parseSizeBytes(text: String): Long? {
    if (text.isBlank()) return null
    val match = Regex("(\\d+(?:\\.\\d+)?)\\s*(TB|GB|MB|KB)", RegexOption.IGNORE_CASE)
        .find(text) ?: return null
    val amount = match.groupValues[1].toDoubleOrNull() ?: return null
    val unit = when (match.groupValues[2].uppercase()) {
        "TB" -> 1_099_511_627_776.0
        "GB" -> 1_073_741_824.0
        "MB" -> 1_048_576.0
        else -> 1_024.0
    }
    return (amount * unit).toLong()
}
