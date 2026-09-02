package com.godiegh.clipx

import android.app.NotificationManager
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.widget.Toast
import com.godiegh.clipx.ffi.BridgeService
import com.godiegh.clipx.ffi.ClipxEventListener
import com.godiegh.clipx.ffi.ClipboardPlatform
import com.godiegh.clipx.ffi.NotificationPlatform
import com.godiegh.clipx.ffi.NotifierDecision
import com.godiegh.clipx.ui.ClipxSheetAction
import com.godiegh.clipx.ui.ClipxSheetRequest
import com.godiegh.clipx.ui.ClipxSheetResult
import com.godiegh.clipx.ui.ClipxSheetResultType
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/** System clipboard adapter used by core for *incoming* clipboard decisions. */
class AndroidClipboardPlatform(
    private val context: Context,
    private val bridgeService: BridgeService,
) : ClipboardPlatform {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val clipboardManager = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

    private val listener = ClipboardManager.OnPrimaryClipChangedListener {
        if (!(context.applicationContext as ClipxApplication).isActivityVisible) return@OnPrimaryClipChangedListener
        currentClipboardText()?.let(::reportIfNewLocalChange)
    }

    private fun currentClipboardText(): String? {
        val clip = clipboardManager.primaryClip ?: return null
        if (clip.itemCount == 0) return null
        return clip.getItemAt(0).coerceToText(context)?.toString()
    }

    private fun reportIfNewLocalChange(content: String) {
        val suppressed = globalSuppressed.getAndSet(null)
        if (suppressed == content) return
        scope.launch { bridgeService.reportClipboardChanged(content) }
    }

    /**
     * Explicit, single-shot check for when ClipX's Activity becomes active
     * again (foreground return, or a floating window over ClipX being
     * dismissed). The system listener only fires on a clipboard-changed
     * event while we're registered/visible, so a copy made in another app
     * while we were backgrounded is otherwise never observed. This is not
     * polling — it runs once per foreground transition, driven by the
     * Activity lifecycle callback in ClipxApplication.
     */
    fun checkClipboardNow() {
        currentClipboardText()?.let(::reportIfNewLocalChange)
    }

    override fun writeClipboard(content: String) {
        // Core calls this only for an accepted incoming item. The core's
        // last_known_content guard suppresses the resulting callback.
        clipboardManager.setPrimaryClip(ClipData.newPlainText("Clipx", content))
        Handler(Looper.getMainLooper()).post {
            Toast.makeText(context.applicationContext, "Copied to clipboard", Toast.LENGTH_SHORT).show()
        }
    }

    fun startListening() {
        clipboardManager.addPrimaryClipChangedListener(listener)
    }

    fun stopListening() {
        clipboardManager.removePrimaryClipChangedListener(listener)
        scope.cancel()
    }

    companion object {
        private val globalSuppressed = AtomicReference<String?>(null)

        /** UI history copy: write to Android directly without feeding it back to core. */
        fun copyWithoutSync(context: Context, content: String) {
            globalSuppressed.set(content)
            val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("Clipx", content))
        }
    }
}

/** Android presentation adapter for interactive core prompts. */
class AndroidNotificationPlatform(
    private val context: Context,
    private val bridgeService: BridgeService,
) : NotificationPlatform {
    private val mainHandler = Handler(Looper.getMainLooper())
    private val notificationManager =
        context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager

    private fun resolve(promptId: String, decision: NotifierDecision) {
        bridgeService.resolvePrompt(promptId, decision)
    }

    private fun showSheet(
        request: ClipxSheetRequest,
        promptId: String,
        onDecision: (ClipxSheetResult) -> NotifierDecision,
    ) {
        mainHandler.post {
            val controller = (context.applicationContext as ClipxApplication).sheetController
            controller.show(request) { result ->
                resolve(promptId, onDecision(result))
            }
        }
    }

    private fun showBackgroundNotification(
        promptId: String,
        title: String,
        message: String,
        actions: List<Pair<String, Int>>,
    ) {
        val intent = Intent(context, ClipxNotificationActionReceiver::class.java).apply {
            putExtra(ClipxNotificationActionReceiver.EXTRA_PROMPT_ID, promptId)
        }
        val builder = androidx.core.app.NotificationCompat.Builder(context, ClipxCoreForegroundService.PROMPT_CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_notification_clipx)
            .setContentTitle(title)
            .setContentText(message)
            .setAutoCancel(true)
            .setPriority(androidx.core.app.NotificationCompat.PRIORITY_HIGH)

        actions.forEachIndexed { index, (action, decisionKind) ->
            val actionIntent = Intent(intent).apply {
                putExtra(ClipxNotificationActionReceiver.EXTRA_DECISION, decisionKind)
            }
            val pending = android.app.PendingIntent.getBroadcast(
                context,
                (promptId + index).hashCode(),
                actionIntent,
                android.app.PendingIntent.FLAG_UPDATE_CURRENT or android.app.PendingIntent.FLAG_IMMUTABLE,
            )
            builder.addAction(0, action, pending)
        }

        notificationManager.notify(promptId.hashCode(), builder.build())
    }

    private fun present(
        promptId: String,
        request: ClipxSheetRequest,
        backgroundTitle: String = request.title,
        backgroundMessage: String = request.message.orEmpty(),
        onDecision: (ClipxSheetResult) -> NotifierDecision,
        backgroundActions: List<Pair<String, Int>>,
    ) {
        val app = context.applicationContext as ClipxApplication
        if (app.isActivityVisible) {
            showSheet(request, promptId, onDecision)
        } else {
            showBackgroundNotification(promptId, backgroundTitle, backgroundMessage, backgroundActions)
        }
    }

    override fun showPairRequest(promptId: String, deviceName: String) {
        
        present(
            promptId,
            ClipxSheetRequest(
                title = "Pairing request",
                message = "$deviceName wants to pair with this device.",
                actions = listOf(
                    ClipxSheetAction("allow", "Allow"),
                    ClipxSheetAction("deny", "Deny", destructive = true),
                ),
            ),
            onDecision = { result ->
                NotifierDecision.PairDecision(
                    when {
                        result.type != ClipxSheetResultType.ACTION -> 2u
                        result.actionId == "allow" -> 0u
                        else -> 1u
                    },
                )
            },
            backgroundActions = listOf("Allow" to 0, "Deny" to 1),
        )
    }

    override fun showPairCode(promptId: String, deviceName: String, code: String) {
        
        present(
            promptId,
            ClipxSheetRequest(
                title = "Confirm pairing code",
                message = "Code from $deviceName: $code\nDoes this match on both devices?",
                actions = listOf(
                    ClipxSheetAction("allow", "Confirm"),
                    ClipxSheetAction("deny", "Deny", destructive = true),
                ),
            ),
            onDecision = { result ->
                NotifierDecision.PairDecision(
                    when {
                        result.type != ClipxSheetResultType.ACTION -> 2u
                        result.actionId == "allow" -> 0u
                        else -> 1u
                    },
                )
            },
            backgroundActions = listOf("Confirm" to 0, "Deny" to 1),
        )
    }

    override fun showReceivedClipboard(promptId: String, deviceName: String) {
        
        present(
            promptId,
            ClipxSheetRequest(
                title = "Clipboard received",
                message = "Received clipboard from $deviceName. Would you like to copy it to your clipboard?",
                actions = listOf(ClipxSheetAction("copy", "Copy to clipboard")),
            ),
            onDecision = { result ->
                NotifierDecision.IncomingClipboardDecision(
                    if (result.type == ClipxSheetResultType.ACTION && result.actionId == "copy") 0u else 1u,
                )
            },
            backgroundActions = listOf("Copy to clipboard" to 2, "Dismiss" to 3),
        )
    }

    override fun notifyInfo(title: String, message: String) {
        mainHandler.post {
            Toast.makeText(context.applicationContext, "$title\n$message", Toast.LENGTH_LONG).show()
        }
    }
}

class AndroidCoreEventListener(
    private val application: ClipxApplication,
) : ClipxEventListener {
    override fun onDeviceChange() {
        application.publishCoreEvent(CoreUiEvent.DevicesChanged)
    }

    override fun onClipboardChange() {
        application.publishCoreEvent(CoreUiEvent.ClipboardChanged)
    }

    override fun onPairingChange(deviceId: String, state: UByte, message: String) {
        application.publishCoreEvent(CoreUiEvent.PairingChanged(deviceId, state.toInt(), message))
    }
}

class ClipxNotificationActionReceiver : android.content.BroadcastReceiver() {
    companion object {
        const val EXTRA_PROMPT_ID = "prompt_id"
        const val EXTRA_DECISION = "decision"
    }

    override fun onReceive(context: Context, intent: Intent) {
        val promptId = intent.getStringExtra(EXTRA_PROMPT_ID) ?: return
        val decisionKind = intent.getIntExtra(EXTRA_DECISION, -1)
        val decision = when (decisionKind) {
            0 -> NotifierDecision.PairDecision(0u)
            1 -> NotifierDecision.PairDecision(1u)
            2 -> NotifierDecision.IncomingClipboardDecision(0u)
            3 -> NotifierDecision.IncomingClipboardDecision(1u)
            else -> return
        }
        val service = (context.applicationContext as ClipxApplication).bridgeService ?: return
        service.resolvePrompt(promptId, decision)
        val notificationManager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        notificationManager.cancel(promptId.hashCode())
    }
}
