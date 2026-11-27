package com.senzo.app.plugins

import java.security.MessageDigest
import java.security.SecureRandom
import java.util.Locale
import javax.crypto.Cipher
import javax.crypto.Mac
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.IvParameterSpec
import javax.crypto.spec.SecretKeySpec

/**
 * The crypto a provider script can reach.
 *
 * Values cross the bridge as hex, exactly like the desktop runtime, so the
 * same JavaScript shims work on both platforms.
 */
object PluginCrypto {

    fun digest(hash: String, data: String): String {
        val name = jcaName(hash) ?: throw IllegalArgumentException("Unsupported hash algorithm.")
        val digest = MessageDigest.getInstance(name)
        return digest.digest(decode(data)).toHex()
    }

    fun hmac(hash: String, key: String, data: String): String {
        val name = jcaName(hash) ?: throw IllegalArgumentException("Unsupported HMAC algorithm.")
        val mac = Mac.getInstance(name)
        mac.init(SecretKeySpec(decode(key), name))
        return mac.doFinal(decode(data)).toHex()
    }

    fun pbkdf2(password: String, salt: String, iterations: Int, bits: Int, hash: String): String {
        val name = jcaName(hash) ?: throw IllegalArgumentException("Unsupported PBKDF2 hash.")
        if (iterations <= 0 || iterations > 1_000_000) {
            throw IllegalArgumentException("PBKDF2 iteration count is outside the allowed range.")
        }
        if (bits <= 0 || bits > 16_384 || bits % 8 != 0) {
            throw IllegalArgumentException("PBKDF2 key size is outside the allowed range.")
        }
        val mac = Mac.getInstance(name)
        mac.init(SecretKeySpec(decode(password), name))
        val saltBytes = decode(salt)
        val wanted = bits / 8
        val output = ByteArray(wanted)
        var block = 1
        var written = 0
        while (written < wanted) {
            mac.reset()
            mac.update(saltBytes)
            mac.update(byteArrayOf((block ushr 24).toByte(), (block ushr 16).toByte(), (block ushr 8).toByte(), block.toByte()))
            var chunk = mac.doFinal()
            val accumulator = chunk.copyOf()
            for (round in 1 until iterations) {
                chunk = mac.doFinal(chunk)
                for (index in accumulator.indices) {
                    accumulator[index] = (accumulator[index].toInt() xor chunk[index].toInt()).toByte()
                }
            }
            val take = minOf(accumulator.size, wanted - written)
            accumulator.copyInto(output, written, 0, take)
            written += take
            block += 1
        }
        return output.toHex()
    }

    fun aes(encrypt: Boolean, mode: String, key: String, iv: String, data: String): String {
        val noPadding = mode.endsWith("-NoPadding")
        val cipherName = when (val base = mode.removeSuffix("-NoPadding")) {
            "AES-CBC" -> if (noPadding) "AES/CBC/NoPadding" else "AES/CBC/PKCS5Padding"
            "AES-ECB" -> if (noPadding) "AES/ECB/NoPadding" else "AES/ECB/PKCS5Padding"
            "AES-GCM" -> "AES/GCM/NoPadding"
            else -> throw IllegalArgumentException("Unsupported AES mode.")
        }
        val keyBytes = decode(key)
        if (keyBytes.size != 16 && keyBytes.size != 24 && keyBytes.size != 32) {
            throw IllegalArgumentException("AES keys must be 16, 24, or 32 bytes.")
        }
        val cipher = Cipher.getInstance(cipherName)
        val ivBytes = decode(iv)
        val secret = SecretKeySpec(keyBytes, "AES")
        when {
            cipherName.startsWith("AES/GCM") -> {
                if (ivBytes.size != 12) {
                    throw IllegalArgumentException("AES-GCM currently requires a 12-byte nonce.")
                }
                cipher.init(
                    if (encrypt) Cipher.ENCRYPT_MODE else Cipher.DECRYPT_MODE,
                    secret,
                    GCMParameterSpec(128, ivBytes),
                )
            }
            cipherName.startsWith("AES/ECB") -> {
                cipher.init(if (encrypt) Cipher.ENCRYPT_MODE else Cipher.DECRYPT_MODE, secret)
            }
            else -> {
                if (ivBytes.size != 16) {
                    throw IllegalArgumentException("AES-CBC requires a 16-byte IV.")
                }
                cipher.init(
                    if (encrypt) Cipher.ENCRYPT_MODE else Cipher.DECRYPT_MODE,
                    secret,
                    IvParameterSpec(ivBytes),
                )
            }
        }
        val result = cipher.doFinal(decode(data))
        return result.toHex()
    }

    fun randomHex(length: Int): String {
        val size = length.coerceIn(0, 1024 * 1024)
        val bytes = ByteArray(size)
        SecureRandom().nextBytes(bytes)
        return bytes.toHex()
    }

    private fun jcaName(name: String): String? = when (
        name.filter { it.isLetterOrDigit() }.uppercase(Locale.US)
    ) {
        "MD5" -> "MD5"
        "SHA1" -> "SHA-1"
        "SHA256" -> "SHA-256"
        "SHA384" -> "SHA-384"
        "SHA512" -> "SHA-512"
        else -> null
    }

    private fun decode(value: String): ByteArray {
        if (value.isEmpty()) return ByteArray(0)
        val clean = value.trim()
        if (clean.length % 2 != 0) throw IllegalArgumentException("Invalid hex input.")
        val output = ByteArray(clean.length / 2)
        for (index in output.indices) {
            val high = Character.digit(clean[index * 2], 16)
            val low = Character.digit(clean[index * 2 + 1], 16)
            if (high < 0 || low < 0) throw IllegalArgumentException("Invalid hex input.")
            output[index] = ((high shl 4) or low).toByte()
        }
        return output
    }

    private fun ByteArray.toHex(): String {
        val digits = "0123456789abcdef"
        val builder = StringBuilder(size * 2)
        for (byte in this) {
            val value = byte.toInt() and 0xFF
            builder.append(digits[value ushr 4]).append(digits[value and 0x0F])
        }
        return builder.toString()
    }
}
