package com.godiegh.clipx

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.widget.Toast
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import com.godiegh.clipx.ffi.ClipxEventListener
import com.godiegh.clipx.ffi.ClipboardPlatform
import com.godiegh.clipx.ffi.NotificationPlatform
import com.godiegh.clipx.ffi.NotifierDecision
import com.godiegh.clipx.ui.ClipxSheetAction
import com.godiegh.clipx.ui.ClipxSheetRequest
import com.godiegh.clipx.ui.ClipxSheetResult
import com.godiegh.clipx.ui.ClipxSheetResultType

class AndroidClipboardPlatform(
    private val context: Context,
    private val bridgeService: com.godiegh.clipx.ffi.BridgeService,
): ClipboardPlatform {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val clipboardManager = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    private val listener = ClipboardManager.OnPrimaryClipChangedListener {
        val clip = clipboardManager.primaryClip ?: return@OnPrimaryClipChangedListener
        val item = clip.getItemAt(0)
        val content = item.coerceToText(context)?.toString() ?: return@OnPrimaryClipChangedListener
        scope.launch { bridgeService.reportClipboardChanged(content) }
    }

    override fun writeClipboard(content: String) {
        clipboardManager.setPrimaryClip(ClipData.newPlainText("Clipx", content))
    }

    fun startListening() {
        clipboardManager.addPrimaryClipChangedListener(listener)
    }

    fun stopListening() {
        clipboardManager.removePrimaryClipChangedListener(listener)
        scope.cancel()
    }
}

class AndroidNotificationPlatform(
    private val context: Context,
    private val bridgeService: com.godiegh.clipx.ffi.BridgeService,
) : NotificationPlatform {
    private val mainHandler = Handler(Looper.getMainLooper())

    private fun showSheet(
        request: ClipxSheetRequest,
        promptId: String,
        onDecision: (ClipxSheetResult) -> NotifierDecision,
    ) {
        mainHandler.post {
            val controller = (context.applicationContext as ClipxApplication).sheetController
            controller.show(request) { result ->
                bridgeService.resolvePrompt(promptId, onDecision(result))
            }
        }
    }

    override fun showPairRequest(promptId: String, deviceName: String) {
        showSheet(
            ClipxSheetRequest(
                title = "Pairing request",
                message = "$deviceName wants to pair with this device.",
                actions = listOf(
                    ClipxSheetAction("allow", "Allow"),
                    ClipxSheetAction("deny", "Deny", destructive = true),
                ),
            ),
            promptId,
        ) { result ->
            NotifierDecision.PairDecision(
                when {
                    result.type != ClipxSheetResultType.ACTION -> 2u
                    result.actionId == "allow" -> 0u
                    else -> 1u
                },
            )
        }
    }

    override fun showPairCode(promptId: String, deviceName: String, code: String) {
        showSheet(
            ClipxSheetRequest(
                title = "Confirm pairing code",
                message = "Code from $deviceName: $code\nDoes this match on both devices?",
                actions = listOf(
                    ClipxSheetAction("allow", "Confirm"),
                    ClipxSheetAction("deny", "Deny", destructive = true),
                ),
            ),
            promptId,
        ) { result ->
            NotifierDecision.PairDecision(
                when {
                    result.type != ClipxSheetResultType.ACTION -> 2u
                    result.actionId == "allow" -> 0u
                    else -> 1u
                },
            )
        }
    }

    override fun showReceivedClipboard(promptId: String, deviceName: String) {
        showSheet(
            ClipxSheetRequest(
                title = "Clipboard received",
                message = "Received clipboard from $deviceName. Would you like to copy it to your clipboard?",
                actions = listOf(ClipxSheetAction("copy", "Copy to clipboard")),
            ),
            promptId,
        ) { result ->
            NotifierDecision.IncomingClipboardDecision(
                if (result.type == ClipxSheetResultType.ACTION && result.actionId == "copy") 0u else 1u,
            )
        }
    }

    override fun notifyInfo(title: String, message: String) {
        mainHandler.post {
            Toast.makeText(context.applicationContext, "$title\n$message", Toast.LENGTH_LONG).show()
        }
    }
}

class AndroidCoreEventListener : ClipxEventListener {
    override fun onDeviceChange() = Unit
    override fun onClipboardChange() = Unit
}
