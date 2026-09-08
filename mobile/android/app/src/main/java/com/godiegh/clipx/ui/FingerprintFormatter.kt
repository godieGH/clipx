package com.godiegh.clipx.ui

/**
 * Display-only fingerprint formatter.
 *
 * Clipx fingerprints are 32-byte identifiers represented internally as 64 hex characters.
 * The UI intentionally shows only the first 28 hex characters as seven groups of four.
 */
fun formatFingerprint(raw: String): String {
    val hex = raw.filter { it.isDigit() || it.lowercaseChar() in 'a'..'f' }
    val display = hex.take(28)
    return display.chunked(4).joinToString("-")
}
