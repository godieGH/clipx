package com.godiegh.clipx

import android.app.Activity
import android.app.Application
import android.os.Bundle
import com.godiegh.clipx.ffi.BridgeService
import com.godiegh.clipx.ffi.initLogging
import com.godiegh.clipx.ui.ClipxSheetController
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first

/**
 * Android application singleton for Clipx.
 *
 * This class owns the app-wide bridge lifecycle, activity visibility state, and
 * the event stream that the Compose UI listens to for device and clipboard
 * updates. It acts as the host-side coordinator between the Rust core and the
 * Android presentation layer.
 */
class ClipxApplication : Application() {
    val sheetController = ClipxSheetController()
    val transferNotifications by lazy { AndroidTransferNotificationController(this) }


    private val unreadClipboardPromptIds = mutableSetOf<String>()

    @Synchronized
    fun markClipboardPromptUnread(promptId: String): Int {
        unreadClipboardPromptIds.add(promptId)
        return unreadClipboardPromptIds.size
    }

    @Synchronized
    fun clearClipboardPrompt(promptId: String): Int {
        unreadClipboardPromptIds.remove(promptId)
        return unreadClipboardPromptIds.size
    }
    @Volatile
    var bridgeService: BridgeService? = null
        private set

    private val _bridge = MutableStateFlow<BridgeService?>(null)

    @Volatile
    var isActivityVisible: Boolean = false
        private set

    /** Set by ClipxCoreForegroundService once clipboardPlatform exists; triggers
     *  a one-shot clipboard check whenever an Activity becomes active again. */
    @Volatile
    var onActiveClipboardCheck: (() -> Unit)? = null

    private val _coreEvents = MutableSharedFlow<CoreUiEvent>(extraBufferCapacity = 32)
    val coreEvents: SharedFlow<CoreUiEvent> = _coreEvents.asSharedFlow()

    fun publishCoreEvent(event: CoreUiEvent) {
        _coreEvents.tryEmit(event)
    }

    fun setBridgeService(service: BridgeService?) {
        bridgeService = service
        _bridge.value = service
    }

    suspend fun awaitBridgeService(): BridgeService = _bridge.filterNotNull().first()

    override fun onCreate() {
        super.onCreate()
        initLogging()
        registerActivityLifecycleCallbacks(object : ActivityLifecycleCallbacks {
            override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) = Unit
            override fun onActivityStarted(activity: Activity) { isActivityVisible = true }
            override fun onActivityResumed(activity: Activity) {
                // A resume covers both "returned from background" (stop -> start
                // -> resume) and "a floating window over us got dismissed"
                // (pause -> resume, no stop/start in between). One clipboard
                // read here, not a poll loop — redundant reads are harmless
                // since both the local suppression guard and the core's
                // last_known_content check no-op on unchanged content.
                isActivityVisible = true
                onActiveClipboardCheck?.invoke()
            }
            override fun onActivityPaused(activity: Activity) = Unit
            override fun onActivityStopped(activity: Activity) { isActivityVisible = false }
            override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit
            override fun onActivityDestroyed(activity: Activity) = Unit
        })
    }
}

/**
 * Events pushed from the Rust core into the Android UI state layer.
 *
 * These are converted into Compose state updates such as refreshing paired
 * devices, update clipboard history, or showing file-transfer progress.
 */
sealed interface CoreUiEvent {
    data object DevicesChanged : CoreUiEvent
    data object ClipboardChanged : CoreUiEvent
    data class PairingChanged(val deviceId: String, val state: Int, val message: String) : CoreUiEvent
    data class FileTransferChanged(val entryId: String, val fileId: String, val fileName: String, val direction: String, val done: Long, val total: Long, val state: String, val message: String) : CoreUiEvent
}
