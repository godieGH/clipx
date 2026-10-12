package com.godiegh.clipx

internal fun shouldHostPeer(ownFingerprint: String, peerFingerprint: String): Boolean =
    ownFingerprint > peerFingerprint

internal fun deriveDirectCredentials(ownFingerprint: String, peerFingerprint: String): Pair<String, String> {
    val (a, b) = if (ownFingerprint < peerFingerprint) {
        ownFingerprint to peerFingerprint
    } else {
        peerFingerprint to ownFingerprint
    }
    val hash = sha256Hex("clipx-p2p|$a|$b")
    return "DIRECT-cx-${hash.substring(0, 8)}" to hash.substring(8, 40)
}

internal fun normalizeDeviceAddress(deviceAddress: String?): String? {
    val cleaned = deviceAddress?.trim() ?: return null
    if (cleaned.isEmpty()) return null
    val normalized = cleaned.replace('-', ':').lowercase()
    if (!Regex("^([0-9a-f]{2}:){5}[0-9a-f]{2}$").matches(normalized)) return null
    return normalized
}

internal fun sha256Hex(text: String): String {
    val bytes = java.security.MessageDigest.getInstance("SHA-256").digest(text.toByteArray())
    return bytes.joinToString("") { "%02x".format(it) }
}
