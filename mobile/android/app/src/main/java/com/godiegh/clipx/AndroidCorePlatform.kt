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
import androidx.core.content.edit
import kotlinx.coroutines.sync.withLock

/** System clipboard adapter used by core for *incoming* clipboard decisions. */
class AndroidClipboardPlatform(
    private val context: Context,
    private val bridgeService: BridgeService,
) : ClipboardPlatform {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val clipboardManager = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

    private val listener = ClipboardManager.OnPrimaryClipChangedListener {
        val app = context.applicationContext as ClipxApplication
        if (!app.isActivityVisible || !isAutoSyncOnResumeEnabled(context)) return@OnPrimaryClipChangedListener
        scope.launch { sendCurrentClipboardIfNeeded(force = false) }
    }

    private val syncMutex = kotlinx.coroutines.sync.Mutex()

    private suspend fun sendCurrentClipboardIfNeeded(force: Boolean) = syncMutex.withLock {
        val items = readPrimaryClipboard(context)
        for (item in items) {
            if (!item.canSend) continue
            if (item.isText && globalSuppressed.getAndSet(null) == item.text) continue
            val marker = markerFor(item)
            if (!force && marker == getLastSyncedMarker(context)) continue
            runCatching { sendItem(item) }
                .onSuccess { setLastSyncedMarker(context, marker) }
                .onFailure { showError(it.message ?: "Could not auto-sync clipboard") }
        }
    }

    private suspend fun sendItem(item: SystemClipboardItem) {
        when {
            item.isFile -> {
                val uri = item.fileUri?.let(android.net.Uri::parse) ?: return
                val staged = java.io.File(context.cacheDir, "clipx-auto-${java.util.UUID.randomUUID()}")
                context.contentResolver.openInputStream(uri)?.use { input -> staged.outputStream().use { output -> input.copyTo(output, 256 * 1024) } }
                    ?: throw IllegalStateException("Unable to read clipboard file")
                try {
                    bridgeService.sendFilePath(item.fileName ?: "clipboard-file", item.fileMimeType ?: "application/octet-stream", staged.absolutePath)
                } finally {
                    staged.delete()
                }
            }
            item.isImage -> {
                val uri = item.fileUri?.let(android.net.Uri::parse)
                if (uri != null) {
                    val image = decodeImage(context, uri) ?: throw IllegalStateException("Unable to read clipboard image")
                    bridgeService.sendImage(image.first.toUInt(), image.second.toUInt(), image.third)
                } else {
                    throw IllegalStateException("Unable to read clipboard image")
                }
            }
            item.isRichText -> bridgeService.sendRichText(item.text.orEmpty(), item.htmlText.orEmpty())
            item.isText -> bridgeService.sendClipboard(item.text.orEmpty())
        }
    }

    private fun markerFor(item: SystemClipboardItem): String = when {
        item.isFile -> "file:${item.fileUri}:${item.fileSize}"
        item.isImage -> "image:${item.fileUri ?: item.mimeTypes.joinToString()}:${item.fileSize}"
        item.isRichText -> "rich:${item.text}:${item.htmlText}"
        else -> "text:${item.text}"
    }

    private fun showError(message: String) {
        Handler(Looper.getMainLooper()).post {
            Toast.makeText(context.applicationContext, message, Toast.LENGTH_SHORT).show()
        }
    }

    /** Explicit, single-shot foreground check. This bypasses the core's local-change dedupe. */
    fun checkClipboardNow() {
        if (!isAutoSyncOnResumeEnabled(context)) return
        scope.launch { sendCurrentClipboardIfNeeded(force = false) }
    }

    override fun writeClipboard(content: String) {
        clipboardManager.setPrimaryClip(ClipData.newPlainText("Clipx", content))
        Handler(Looper.getMainLooper()).post {
            Toast.makeText(context.applicationContext, "Copied to clipboard", Toast.LENGTH_SHORT).show()
        }
    }

    override fun writeRichText(text: String, html: String) {
        clipboardManager.setPrimaryClip(ClipData.newHtmlText("Clipx", text, html))
    }

    override fun writeImage(width: UInt, height: UInt, rgba: ByteArray) {
        val widthPx = width.toInt()
        val heightPx = height.toInt()
        val bitmap = android.graphics.Bitmap.createBitmap(widthPx, heightPx, android.graphics.Bitmap.Config.ARGB_8888)
        val pixels = IntArray(widthPx * heightPx)
        var offset = 0
        for (i in pixels.indices) {
            val r = rgba.getOrElse(offset) { 0 }.toInt() and 0xFF
            val g = rgba.getOrElse(offset + 1) { 0 }.toInt() and 0xFF
            val b = rgba.getOrElse(offset + 2) { 0 }.toInt() and 0xFF
            val a = rgba.getOrElse(offset + 3) { 0xFF.toByte() }.toInt() and 0xFF
            pixels[i] = android.graphics.Color.argb(a, r, g, b)
            offset += 4
        }
        bitmap.setPixels(pixels, 0, widthPx, 0, 0, widthPx, heightPx)
        val dir = java.io.File(context.cacheDir, "clipboard-images").apply { mkdirs() }
        val imageFile = java.io.File(dir, "incoming-${System.currentTimeMillis()}.png")
        imageFile.outputStream().use { out -> bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, out) }
        bitmap.recycle()
        val uri = androidx.core.content.FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", imageFile)
        clipboardManager.setPrimaryClip(ClipData.newUri(context.contentResolver, "Clipx", uri))
        context.grantUriPermission(context.packageName, uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }

    override fun saveFile(name: String, mimeType: String, data: ByteArray): String {
        val safeName = name.replace(Regex("[\\\\/:*?\"<>|]"), "_").ifBlank { "clipx-file" }
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.Q) {
            val values = android.content.ContentValues().apply {
                put(android.provider.MediaStore.MediaColumns.DISPLAY_NAME, safeName)
                put(android.provider.MediaStore.MediaColumns.MIME_TYPE, mimeType.ifBlank { "application/octet-stream" })
                put(android.provider.MediaStore.MediaColumns.RELATIVE_PATH, "${android.os.Environment.DIRECTORY_DOWNLOADS}/Clipx")
                put(android.provider.MediaStore.MediaColumns.IS_PENDING, 1)
            }
            val uri = context.contentResolver.insert(android.provider.MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
                ?: throw IllegalStateException("Unable to create download")
            try {
                context.contentResolver.openOutputStream(uri)?.use { it.write(data) }
                    ?: throw IllegalStateException("Unable to write download")
                values.clear()
                values.put(android.provider.MediaStore.MediaColumns.IS_PENDING, 0)
                context.contentResolver.update(uri, values, null, null)
                return uri.toString()
            } catch (t: Throwable) {
                context.contentResolver.delete(uri, null, null)
                throw t
            }
        }
        val dir = java.io.File(context.getExternalFilesDir(android.os.Environment.DIRECTORY_DOWNLOADS), "Clipx").apply { mkdirs() }
        val file = java.io.File(dir, safeName)
        file.outputStream().use { it.write(data) }
        return file.absolutePath
    }

    override fun saveFileFromPath(name: String, mimeType: String, sourcePath: String): String {
        val safeName = name.replace(Regex("[\\/:*?\"<>|]"), "_").ifBlank { "clipx-file" }
        val source = java.io.File(sourcePath)
        if (!source.isFile) throw IllegalStateException("Source file is unavailable")
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.Q) {
            val values = android.content.ContentValues().apply {
                put(android.provider.MediaStore.MediaColumns.DISPLAY_NAME, safeName)
                put(android.provider.MediaStore.MediaColumns.MIME_TYPE, mimeType.ifBlank { "application/octet-stream" })
                put(android.provider.MediaStore.MediaColumns.RELATIVE_PATH, "${android.os.Environment.DIRECTORY_DOWNLOADS}/Clipx")
                put(android.provider.MediaStore.MediaColumns.IS_PENDING, 1)
            }
            val uri = context.contentResolver.insert(android.provider.MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
                ?: throw IllegalStateException("Unable to create download")
            try {
                context.contentResolver.openOutputStream(uri)?.use { output ->
                    source.inputStream().use { input -> input.copyTo(output, 256 * 1024) }
                } ?: throw IllegalStateException("Unable to write download")
                values.clear(); values.put(android.provider.MediaStore.MediaColumns.IS_PENDING, 0)
                context.contentResolver.update(uri, values, null, null)
                return uri.toString()
            } catch (t: Throwable) {
                context.contentResolver.delete(uri, null, null)
                throw t
            }
        }
        val dir = java.io.File(context.getExternalFilesDir(android.os.Environment.DIRECTORY_DOWNLOADS), "Clipx").apply { mkdirs() }
        val file = java.io.File(dir, safeName)
        source.copyTo(file, overwrite = false)
        return file.absolutePath
    }

    fun startListening() {
        clipboardManager.addPrimaryClipChangedListener(listener)
    }

    fun stopListening() {
        clipboardManager.removePrimaryClipChangedListener(listener)
        scope.cancel()
    }

    companion object {
        private const val PREFS_NAME = "clipboard_sync"
        private const val KEY_AUTO_SYNC_ON_RESUME = "auto_sync_on_resume"
        private val globalSuppressed = AtomicReference<String?>(null)

        fun readPrimaryClipboard(context: Context): List<SystemClipboardItem> {
            val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            val clip = clipboard.primaryClip ?: return emptyList()
            if (clip.itemCount == 0) return emptyList()
            return (0 until clip.itemCount).map { index ->
                val item = clip.getItemAt(index)
                val description = clip.description
                val mimeTypes = if (description == null) emptyList() else {
                    (0 until description.mimeTypeCount).map { description.getMimeType(it) }
                }
                val isImage = mimeTypes.any { it.startsWith("image/", ignoreCase = true) }
                val uri = item.uri
                val isFile = uri != null && (mimeTypes.any { it == "text/uri-list" } || uri.scheme != null && mimeTypes.none { it.startsWith("text/", ignoreCase = true) })
                val text = when {
                    item.text != null -> item.text.toString()
                    isImage || isFile -> null
                    else -> item.coerceToText(context)?.toString()
                }
                val metadata = if (uri != null) fileMetadata(context, uri) else null
                SystemClipboardItem(
                    index = index,
                    text = text,
                    htmlText = item.htmlText,
                    mimeTypes = mimeTypes,
                    isImage = isImage,
                    isText = !text.isNullOrBlank(),
                    fileUri = uri?.toString().takeIf { isImage || isFile },
                    fileName = metadata?.first,
                    fileMimeType = metadata?.second,
                    fileSize = metadata?.third ?: 0L,
                )
            }
        }

        private fun fileMetadata(context: Context, uri: android.net.Uri): Triple<String, String, Long>? {
            var name = "clipboard-file"
            var mime = context.contentResolver.getType(uri) ?: "application/octet-stream"
            var size = 0L
            context.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
                val nameIndex = cursor.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
                val sizeIndex = cursor.getColumnIndex(android.provider.OpenableColumns.SIZE)
                if (cursor.moveToFirst()) {
                    if (nameIndex >= 0) name = cursor.getString(nameIndex) ?: name
                    if (sizeIndex >= 0) size = cursor.getLong(sizeIndex).coerceAtLeast(0L)
                }
            }
            return Triple(name, mime, size)
        }

        private fun decodeImage(context: Context, uri: android.net.Uri): Triple<Int, Int, ByteArray>? {
            val bitmap = context.contentResolver.openInputStream(uri)?.use {
                android.graphics.BitmapFactory.decodeStream(it)
            } ?: return null
            val rgba = ByteArray(bitmap.width * bitmap.height * 4)
            val pixels = IntArray(bitmap.width * bitmap.height)
            bitmap.getPixels(pixels, 0, bitmap.width, 0, 0, bitmap.width, bitmap.height)
            var offset = 0
            for (pixel in pixels) {
                rgba[offset++] = ((pixel shr 16) and 0xFF).toByte()
                rgba[offset++] = ((pixel shr 8) and 0xFF).toByte()
                rgba[offset++] = (pixel and 0xFF).toByte()
                rgba[offset++] = ((pixel ushr 24) and 0xFF).toByte()
            }
            val result = Triple(bitmap.width, bitmap.height, rgba)
            bitmap.recycle()
            return result
        }

        fun isAutoSyncOnResumeEnabled(context: Context): Boolean =
            context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE).getBoolean(KEY_AUTO_SYNC_ON_RESUME, true)

        fun setAutoSyncOnResumeEnabled(context: Context, enabled: Boolean) {
            context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE).edit {
                putBoolean(
                    KEY_AUTO_SYNC_ON_RESUME,
                    enabled
                )
            }
        }

        fun copyWithoutSync(context: Context, content: String) {
            globalSuppressed.set(content)
            val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("Clipx", content))
        }

        private const val KEY_LAST_SYNCED_MARKER = "last_synced_marker"

        fun getLastSyncedMarker(context: Context): String? =
            context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
                .getString(KEY_LAST_SYNCED_MARKER, null)

        fun setLastSyncedMarker(context: Context, marker: String) {
            context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
                .edit { putString(KEY_LAST_SYNCED_MARKER, marker) }
        }

        // make markerFor accessible from outside the class too
        fun markerFor(item: SystemClipboardItem): String = when {
            item.isFile -> "file:${item.fileUri}:${item.fileSize}"
            item.isImage -> "image:${item.fileUri ?: item.mimeTypes.joinToString()}:${item.fileSize}"
            item.isRichText -> "rich:${item.text}:${item.htmlText}"
            else -> "text:${item.text}"
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

        val app = context.applicationContext as ClipxApplication
        builder.setNumber(app.markClipboardPromptUnread(promptId))
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

    override fun showReceivedClipboard(promptId: String, deviceName: String, action: String) {
        present(
            promptId,
            ClipxSheetRequest(
                title = "Clipboard received",
                message = "Received clipboard from $deviceName",
                actions = listOf(ClipxSheetAction("copy", action)),
            ),
            onDecision = { result ->
                NotifierDecision.IncomingClipboardDecision(
                    if (result.type == ClipxSheetResultType.ACTION && result.actionId == "copy") 0u else 1u,
                )
            },
            backgroundActions = listOf(action to 2, "Dismiss" to 3),
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

    override fun onFileTransfer(entryId: String, fileId: String, done: ULong, total: ULong, state: String, message: String) {
        application.publishCoreEvent(CoreUiEvent.FileTransferChanged(entryId, fileId, done.toLong(), total.toLong(), state, message))
        application.transferNotifications.update(fileId, done.toLong(), total.toLong(), state, message)
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
        val app = context.applicationContext as ClipxApplication
        app.clearClipboardPrompt(promptId)
        val notificationManager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        notificationManager.cancel(promptId.hashCode())
    }
}
