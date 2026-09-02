package com.godiegh.clipx

import android.content.Context
import android.content.Intent
import androidx.lifecycle.ViewModel
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.lifecycle.viewModelScope
import com.godiegh.clipx.ffi.BridgeService
import com.godiegh.clipx.ui.ClipxHistoryItem
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.time.Duration.Companion.milliseconds

class ClipxCoreViewModel(
    private val application: ClipxApplication,
) : ViewModel() {
    var pairedDevices by androidx.compose.runtime.mutableStateOf<List<PairedDevice>>(emptyList())
        private set
    var availableDevices by androidx.compose.runtime.mutableStateOf<List<AvailableDevice>>(emptyList())
        private set
    var historyItems by androidx.compose.runtime.mutableStateOf<List<ClipxHistoryItem>>(emptyList())
        private set
    var systemClipboardItem by androidx.compose.runtime.mutableStateOf<SystemClipboardItem?>(null)
        private set
    var autoSyncOnResume by androidx.compose.runtime.mutableStateOf(
        AndroidClipboardPlatform.isAutoSyncOnResumeEnabled(application)
    )
        private set
    var identity by androidx.compose.runtime.mutableStateOf<ThisDeviceInfo?>(null)
        private set
    var requestingPairId by androidx.compose.runtime.mutableStateOf<String?>(null)
        private set
    var loading by androidx.compose.runtime.mutableStateOf(true)
        private set
    var lastError by androidx.compose.runtime.mutableStateOf<String?>(null)
        private set
    var pendingFiles by androidx.compose.runtime.mutableStateOf<List<OutgoingFile>>(emptyList())
        private set
    var sharedSyncRequested by androidx.compose.runtime.mutableStateOf(false)
        private set
    var fileTransfers by androidx.compose.runtime.mutableStateOf<Map<String, FileTransferUi>>(emptyMap())
        private set
    var refreshing by androidx.compose.runtime.mutableStateOf(false)
        private set
    var sendingFiles by androidx.compose.runtime.mutableStateOf(false)
        private set

    private val _messages = MutableSharedFlow<String>(extraBufferCapacity = 16)
    val messages: SharedFlow<String> = _messages.asSharedFlow()

    private var bridge: BridgeService? = null
    private var scanJob: Job? = null

    init {
        viewModelScope.launch {
            bridge = application.awaitBridgeService()
            refreshAll()
            refreshAvailable()
            delay(2_500.milliseconds)
            refreshAvailable()
        }
        viewModelScope.launch {
            application.coreEvents.collect(::handleEvent)
        }
    }

    private suspend fun handleEvent(event: CoreUiEvent) {
        when (event) {
            CoreUiEvent.DevicesChanged -> refreshPaired()
            CoreUiEvent.ClipboardChanged -> refreshHistory()
            is CoreUiEvent.PairingChanged -> {
                refreshPaired()
                refreshAvailable()
                when (event.state) {
                    0 -> requestingPairId = event.deviceId
                    1, 2 -> {
                        if (requestingPairId == event.deviceId) requestingPairId = null
                    }
                }
            }
            is CoreUiEvent.FileTransferChanged -> {
                fileTransfers = fileTransfers + (event.entryId to FileTransferUi(event.fileId, event.done, event.total, event.state, event.message)) + (event.fileId to FileTransferUi(event.fileId, event.done, event.total, event.state, event.message))
            }
        }
    }

    suspend fun refreshAll() {
        loading = true
        refreshIdentity()
        refreshPaired()
        refreshHistory()
        loading = false
    }

    suspend fun refreshIdentity() {
        bridge?.let { service ->
            runCatching { service.getIdentity() }
                .onSuccess { value ->
                    identity = ThisDeviceInfo(
                        id = value.id,
                        name = value.name,
                        fingerprint = value.fingerprint,
                        deviceType = deviceTypeFromString(value.deviceType),
                        ipAddress = value.ipAddress,
                        wsPort = value.wsPort.toInt(),
                    )
                }
                .onFailure { lastError = it.message }
        }
    }

    suspend fun refreshPaired() {
        bridge?.let { service ->
            runCatching { service.getPairedDevices() }
                .onSuccess { list ->
                    pairedDevices = list.map {
                        PairedDevice(
                            id = it.id,
                            name = it.name,
                            type = deviceTypeFromString(it.deviceType),
                            status = connectionStatusFromString(it.connection),
                            ipAddress = it.ipAddress,
                            wsPort = it.wsPort.toInt(),
                            autoConnect = it.autoConnect,
                        )
                    }
                }
                .onFailure { lastError = it.message }
        }
    }

    suspend fun refreshAvailable() {
        bridge?.let { service ->
            runCatching { service.getAvailableDevices() }
                .onSuccess { list ->
                    availableDevices = list
                        .filterNot { candidate -> pairedDevices.any { it.id == candidate.id } }
                        .map {
                            AvailableDevice(
                                id = it.id,
                                name = it.name,
                                type = deviceTypeFromString(it.deviceType),
                                status = PairedStatus.IDLE,
                            )
                        }
                }
                .onFailure { lastError = it.message }
        }
    }

    suspend fun refreshHistory() {
        bridge?.let { service ->
            runCatching { service.getClipboardHistory(0u) }
                .onSuccess { list ->
                    historyItems = list.map {
                        val type = when (it.kind.lowercase()) {
                            "rich_text", "richtext" -> ClipType.RICH_TEXT
                            "image" -> ClipType.IMAGE
                            "file" -> ClipType.FILE
                            else -> ClipType.TEXT
                        }
                        ClipxHistoryItem(
                            id = it.id,
                            preview = it.fileName.ifBlank { it.content },
                            content = it.content,
                            sourceDevice = it.sourceDevice,
                            ageLabel = timeAgo(it.receivedAtMs),
                            type = type,
                            html = it.html.ifBlank { null },
                            fileId = it.fileId.ifBlank { null },
                            fileName = it.fileName.ifBlank { null },
                            mimeType = it.mimeType.ifBlank { null },
                            fileSize = it.fileSize.toLong(),
                            fileExpiresAtMs = it.fileExpiresAtMs.toLong(),
                            fileDownloaded = it.fileDownloaded,
                        )
                    }
                }
                .onFailure { lastError = it.message }
        }
    }

    fun pair(deviceId: String) {
        requestingPairId = deviceId
        viewModelScope.launch {
            bridge?.let { service ->
                runCatching { service.pairDevice(deviceId) }
                    .onSuccess { message ->
                        if (!message.equals("pairing started", ignoreCase = true)) {
                            requestingPairId = null
                            emitMessage(message)
                        }
                    }
                    .onFailure {
                        requestingPairId = null
                        emitMessage(it.message ?: "Pairing failed")
                    }
            }
        }
    }

    fun connect(deviceId: String) {
        viewModelScope.launch {
            bridge?.let { service ->
                runCatching { service.connectDevice(deviceId) }
                    .onSuccess { message ->
                        refreshPaired()
                        if (!message.equals("connecting", ignoreCase = true)) emitMessage(message)
                    }
                    .onFailure { emitMessage(it.message ?: "Unable to connect") }
            }
        }
    }

    fun disconnect(deviceId: String) {
        viewModelScope.launch {
            bridge?.let { service ->
                runCatching { service.disconnectDevice(deviceId) }
                    .onSuccess { message ->
                        refreshPaired()
                        emitMessage(message.replaceFirstChar { it.uppercase() })
                    }
                    .onFailure { emitMessage(it.message ?: "Unable to disconnect") }
            }
        }
    }

    fun setAutoConnect(deviceId: String, enabled: Boolean) {
        viewModelScope.launch {
            bridge?.let { service ->
                runCatching { service.setAutoConnect(deviceId, enabled) }
                    .onSuccess { ok ->
                        if (ok) refreshPaired() else emitMessage("Could not update auto-connect")
                    }
                    .onFailure { emitMessage(it.message ?: "Could not update auto-connect") }
            }
        }
    }

    fun forget(deviceId: String) {
        viewModelScope.launch {
            bridge?.let { service ->
                runCatching { service.forgetDevice(deviceId) }
                    .onSuccess {
                        refreshPaired()
                        emitMessage("Device forgotten")
                    }
                    .onFailure { emitMessage(it.message ?: "Could not forget device") }
            }
        }
    }

    fun refreshSystemClipboard(context: Context) {
        systemClipboardItem = AndroidClipboardPlatform.readPrimaryClipboard(context).firstOrNull { it.canSend }
    }

    fun clearSyncClipboardItem() { systemClipboardItem = null }

    fun enqueueFiles(context: Context, uris: List<android.net.Uri>) {
        viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) {
            val added = uris.mapNotNull { uri ->
                runCatching {
                    var name = "clipx-file"
                    var size = 0L
                    context.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
                        val nameIndex = cursor.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
                        val sizeIndex = cursor.getColumnIndex(android.provider.OpenableColumns.SIZE)
                        if (cursor.moveToFirst()) {
                            if (nameIndex >= 0) name = cursor.getString(nameIndex) ?: name
                            if (sizeIndex >= 0) size = cursor.getLong(sizeIndex).coerceAtLeast(0L)
                        }
                    }
                    val mime = context.contentResolver.getType(uri) ?: "application/octet-stream"
                    OutgoingFile(uri, name, mime, size)
                }.getOrNull()
            }
            withContext(kotlinx.coroutines.Dispatchers.Main) { pendingFiles = pendingFiles + added }
        }
    }

    fun enqueueSharedText(text: String) {
        if (text.isBlank()) return

        systemClipboardItem = SystemClipboardItem(
            -1,
            text,
            null,
            listOf("text/plain"),
            false,
            true
        )

        sharedSyncRequested = true
    }

    fun openSharedSync() { sharedSyncRequested = true }
    fun consumeSharedSyncRequest() { sharedSyncRequested = false }
    fun removePendingFile(index: Int) { pendingFiles = pendingFiles.filterIndexed { i, _ -> i != index } }

    fun connectedDeviceCount(): Int = pairedDevices.count { it.status == ConnectionStatus.CONNECTED }

    fun sendSelectedClipboardItem(item: SystemClipboardItem?, context: Context, onDone: () -> Unit = {}) {
        if (item == null) { emitMessage("Select clipboard content to send"); return }
        if (connectedDeviceCount() == 0) { emitMessage("No connected devices"); return }
        viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) {
            runCatching {
                when {
                    item.isFile -> {
                        val uri = item.fileUri?.let(android.net.Uri::parse) ?: error("Clipboard file is unavailable")
                        sendUriAsFile(context, uri, item.fileName ?: "clipboard-file", item.fileMimeType ?: "application/octet-stream")
                    }
                    item.isImage -> {
                        val uri = item.fileUri?.let(android.net.Uri::parse) ?: error("Clipboard image is unavailable")
                        val image = readImage(context, uri) ?: error("Unable to read clipboard image")
                        bridge?.sendImage(image.first.toUInt(), image.second.toUInt(), image.third) ?: error("Clipx core is not ready")
                    }
                    item.isRichText -> bridge?.sendRichText(item.text.orEmpty(), item.htmlText.orEmpty()) ?: error("Clipx core is not ready")
                    item.isText -> bridge?.sendClipboard(item.text.orEmpty()) ?: error("Clipx core is not ready")
                    else -> error("Clipboard content is not supported")
                }
            }.onSuccess { withContext(kotlinx.coroutines.Dispatchers.Main) { emitMessage("Clipboard sent"); onDone() } }
                .onFailure { withContext(kotlinx.coroutines.Dispatchers.Main) { emitMessage(it.message ?: "Could not send clipboard") } }
        }
    }

    fun sendPendingFiles(context: Context, onDone: () -> Unit = {}) {
        if (pendingFiles.isEmpty()) { emitMessage("Select at least one file"); return }
        if (connectedDeviceCount() == 0) { emitMessage("No connected devices"); return }
        val files = pendingFiles
        sendingFiles = true
        viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) {
            var sent = 0
            for (file in files) {
                val result = runCatching {
                    sendUriAsFile(context, file.uri, file.name, file.mimeType)
                }
                if (result.isSuccess) sent++ else { withContext(kotlinx.coroutines.Dispatchers.Main) { emitMessage(result.exceptionOrNull()?.message ?: "Could not send ${file.name}") }; break }
            }
            withContext(kotlinx.coroutines.Dispatchers.Main) {
                if (sent == files.size) { pendingFiles = emptyList(); emitMessage("${sent} file${if (sent == 1) "" else "s"} offered"); onDone() }
                sendingFiles = false
            }
        }
    }

    private suspend fun sendUriAsFile(context: Context, uri: android.net.Uri, name: String, mimeType: String) {
        val service = bridge ?: error("Clipx core is not ready")
        val stagedDir = java.io.File(context.cacheDir, "clipx-outgoing").apply { mkdirs() }
        val staged = java.io.File(stagedDir, java.util.UUID.randomUUID().toString())
        try {
            context.contentResolver.openInputStream(uri)?.use { input ->
                staged.outputStream().use { output -> input.copyTo(output, 256 * 1024) }
            } ?: error("Unable to read $name")
            service.sendFilePath(name, mimeType, staged.absolutePath)
        } finally {
            staged.delete()
        }
    }

    fun restartServiceAndRefresh(context: Context) {
        if (refreshing) return
        refreshing = true
        viewModelScope.launch {
            try {
                application.setBridgeService(null)
                context.stopService(Intent(context, ClipxCoreForegroundService::class.java))
                ClipxCoreForegroundService.start(context)
                bridge = application.awaitBridgeService()
                refreshSystemClipboard(context)
                refreshAll()
                refreshAvailable()
            } catch (t: Throwable) {
                lastError = t.message ?: "Refresh failed"
            } finally {
                refreshing = false
            }
        }
    }

    fun downloadHistoryFile(item: ClipxHistoryItem) {
        if (item.type != ClipType.FILE) return
        viewModelScope.launch {
            bridge?.let { service ->
                runCatching { service.downloadClipboardFile(item.id) }
                    .onSuccess { emitMessage(it) }
                    .onFailure { emitMessage(it.message ?: "Could not download file") }
            }
        }
    }

    fun setAutoSyncOnResume(context: Context, enabled: Boolean) {
        AndroidClipboardPlatform.setAutoSyncOnResumeEnabled(context, enabled)
        autoSyncOnResume = enabled
    }

    fun sendClipboardItem(content: String) {
        viewModelScope.launch {
            val service = bridge
            if (service == null) {
                emitMessage("Clipx core is not ready")
                return@launch
            }
            runCatching { service.sendClipboard(content) }
                .onSuccess { emitMessage("Clipboard sent") }
                .onFailure { emitMessage(it.message ?: "Could not send clipboard") }
        }
    }

    fun removeHistory(id: String) {
        viewModelScope.launch {
            bridge?.let { service ->
                runCatching { service.removeClipboardEntry(id) }
                    .onSuccess { removed ->
                        if (removed) refreshHistory()
                    }
                    .onFailure { emitMessage(it.message ?: "Could not remove item") }
            }
        }
    }

    fun clearHistory() {
        viewModelScope.launch {
            bridge?.let { service ->
                runCatching { service.clearClipboardHistory() }
                    .onSuccess { refreshHistory() }
                    .onFailure { emitMessage(it.message ?: "Could not clear history") }
            }
        }
    }

    fun startActiveScan() {
        if (scanJob?.isActive == true) return
        scanJob = viewModelScope.launch {
            while (isActive) {
                refreshAvailable()
                delay(2_000.milliseconds)
            }
        }
    }

    fun stopActiveScan() {
        scanJob?.cancel()
        scanJob = null
    }

    fun clearError() {
        lastError = null
    }

    private fun emitMessage(message: String) {
        if (message.isNotBlank()) _messages.tryEmit(message)
    }

    override fun onCleared() {
        stopActiveScan()
        // no need to call the super.onCleared it is annotated @Empty or has
        // code that shouldn't be run when overridden
        // super.onCleared()
    }
}

private fun deviceTypeFromString(value: String): DeviceType = when (value.lowercase()) {
    "windows" -> DeviceType.WINDOWS
    "linux" -> DeviceType.LINUX
    "macos" -> DeviceType.MACOS
    "android" -> DeviceType.ANDROID
    "ios" -> DeviceType.IOS
    else -> DeviceType.UNKNOWN
}

private fun connectionStatusFromString(value: String): ConnectionStatus = when (value.lowercase()) {
    "connected" -> ConnectionStatus.CONNECTED
    "connecting" -> ConnectionStatus.CONNECTING
    "unavailable" -> ConnectionStatus.UNAVAILABLE
    else -> ConnectionStatus.DISCONNECTED
}

private fun timeAgo(timestampMs: ULong): String {
    val seconds = ((System.currentTimeMillis() - timestampMs.toLong()).coerceAtLeast(0L)) / 1_000L
    return when {
        seconds < 60 -> "just now"
        seconds < 3_600 -> "${seconds / 60}m ago"
        seconds < 86_400 -> "${seconds / 3_600}h ago"
        else -> "${seconds / 86_400}d ago"
    }
}


data class FileTransferUi(val fileId: String, val done: Long, val total: Long, val state: String, val message: String)

private fun readImage(context: Context, uri: android.net.Uri): Triple<Int, Int, ByteArray>? {
    val bitmap = context.contentResolver.openInputStream(uri)?.use { android.graphics.BitmapFactory.decodeStream(it) } ?: return null
    val pixels = IntArray(bitmap.width * bitmap.height)
    bitmap.getPixels(pixels, 0, bitmap.width, 0, 0, bitmap.width, bitmap.height)
    val rgba = ByteArray(pixels.size * 4)
    var o = 0
    for (pixel in pixels) {
        rgba[o++] = ((pixel shr 16) and 0xFF).toByte()
        rgba[o++] = ((pixel shr 8) and 0xFF).toByte()
        rgba[o++] = (pixel and 0xFF).toByte()
        rgba[o++] = ((pixel ushr 24) and 0xFF).toByte()
    }
    val result = Triple(bitmap.width, bitmap.height, rgba)
    bitmap.recycle()
    return result
}
