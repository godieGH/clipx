package com.godiegh.clipx

enum class DeviceType {
    UNKNOWN,
    WINDOWS,
    MACOS,
    LINUX,
    ANDROID,
    IOS,
}

enum class ConnectionStatus {
    CONNECTED,
    CONNECTING,
    DISCONNECTED,
    UNAVAILABLE,
}

enum class PairedStatus {
    IDLE,
    REQUESTING,
}

enum class ClipType {
    TEXT,
    IMAGE,
    FILE,
    // for now - more will be added
}

data class PairedDevice(
    val id: String, // the fingerprint
    val name: String,
    val type: DeviceType, // MACOS, WINDOWS, ANDROID, LINUX, IOS, UNKNOWN
    val status: ConnectionStatus, // CONNECTED, CONNECTING, DISCONNECTED, UNAVAILABLE,
    val ipAddress: String,
    val wsPort: Int,
    val autoConnect: Boolean = false
)

data class AvailableDevice(
    val id: String,
    val name: String,
    val type: DeviceType,
    val status: PairedStatus,
)

data class ClipItem(
    val id: String,
    val content: String,
    val sourceDevice: String,
    val timestamp: Long,
    val type: ClipType, // IMAGE, TEXT, FILE, CODE
)
