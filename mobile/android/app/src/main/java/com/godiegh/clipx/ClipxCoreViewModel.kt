package com.godiegh.clipx

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

class ClipxCoreViewModel(
    private val application: ClipxApplication,
) : ViewModel() {
    var pairedDevices by androidx.compose.runtime.mutableStateOf<List<PairedDevice>>(emptyList())
        private set
    var availableDevices by androidx.compose.runtime.mutableStateOf<List<AvailableDevice>>(emptyList())
        private set
    var historyItems by androidx.compose.runtime.mutableStateOf<List<ClipxHistoryItem>>(emptyList())
        private set
    var identity by androidx.compose.runtime.mutableStateOf<ThisDeviceInfo?>(null)
        private set
    var requestingPairId by androidx.compose.runtime.mutableStateOf<String?>(null)
        private set
    var loading by androidx.compose.runtime.mutableStateOf(true)
        private set
    var lastError by androidx.compose.runtime.mutableStateOf<String?>(null)
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
            delay(2_500)
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
                    1 -> {
                        if (requestingPairId == event.deviceId) requestingPairId = null
                    }
                    2 -> {
                        if (requestingPairId == event.deviceId) requestingPairId = null
                    }
                }
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
                        ClipxHistoryItem(
                            id = it.id,
                            preview = it.content,
                            content = it.content,
                            sourceDevice = it.sourceDevice,
                            ageLabel = timeAgo(it.receivedAtMs),
                            type = ClipType.TEXT,
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
                delay(2_000)
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
        super.onCleared()
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
