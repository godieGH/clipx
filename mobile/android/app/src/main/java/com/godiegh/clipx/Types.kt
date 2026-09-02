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
    RICH_TEXT,
    IMAGE,
    FILE,
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

data class ThisDeviceInfo(
    val id: String,
    val name: String,
    val fingerprint: String,
    val deviceType: DeviceType,
    val ipAddress: String,
    val wsPort: Int,
)


data class SystemClipboardItem(
    val index: Int,
    val text: String?,
    val htmlText: String?,
    val mimeTypes: List<String>,
    val isImage: Boolean,
    val isText: Boolean,
    val fileUri: String? = null,
    val fileName: String? = null,
    val fileMimeType: String? = null,
    val fileSize: Long = 0L,
) {
    val isFile: Boolean get() = fileUri != null
    val isRichText: Boolean get() = !htmlText.isNullOrBlank() && isText
    val canSend: Boolean get() = isText || isImage || isFile
}

data class OutgoingFile(
    val uri: android.net.Uri,
    val name: String,
    val mimeType: String,
    val size: Long,
)
