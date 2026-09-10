package com.godiegh.clipx

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.webkit.MimeTypeMap
import androidx.core.app.NotificationCompat
import androidx.core.content.FileProvider
import java.util.concurrent.ConcurrentHashMap

/**
 * Owns the system notification for active file transfers. It is fed directly
 * by core events, so the tray notification and the in-app transfer state cannot
 * drift into separate progress lifecycles.
 */
class AndroidTransferNotificationController(private val context: Context) {
    private val notificationManager =
        context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
    private val titles = ConcurrentHashMap<String, String>()

    private fun notificationId(fileId: String): Int = 2000 + (fileId.hashCode() and 0x7fffffff) % 100000

    private fun titleFromMessage(message: String): String = when {
        message.startsWith("Requesting ") -> message.removePrefix("Requesting ")
        message.startsWith("Receiving ") -> message.removePrefix("Receiving ")
        message.startsWith("Sending ") -> message.removePrefix("Sending ")
        else -> message.ifBlank { "File transfer" }
    }.substringBefore(" · ").ifBlank { "File transfer" }

    /**
     * On a "complete" download event, `message` is whatever
     * ClipboardPlatform.saveFile / saveFileFromPath returned: a
     * content://media/... string on API 29+, or a plain absolute file path
     * on older devices (we write those under getExternalFilesDir). Either
     * way, turn it into a Uri a viewer app can actually open — a bare
     * file:// Uri is blocked by StrictMode past API 24, so the file-path
     * branch has to go through our FileProvider instead.
     */
    private fun openableUriFor(message: String): Uri? {
        if (message.startsWith("content://")) return Uri.parse(message)
        val file = java.io.File(message)
        if (!file.isFile) return null
        return runCatching {
            FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", file)
        }.getOrNull()
    }

    private fun mimeTypeFor(fileName: String): String {
        val ext = fileName.substringAfterLast('.', "").lowercase()
        if (ext.isEmpty()) return "*/*"
        return MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext) ?: "*/*"
    }

    fun update(fileId: String, fileName: String, direction: String, done: Long, total: Long, state: String, message: String) {
        val percent = if (total > 0L) ((done.toDouble() / total.toDouble()) * 100.0).toInt().coerceIn(0, 100) else 0
        val active = state == "requesting" || state == "receiving" || state == "sending" || state == "saving"
        val title = if (active) {
            val resolved = fileName.ifBlank { titleFromMessage(message) }
            titles.putIfAbsent(fileId, resolved) ?: resolved
        } else {
            titles.remove(fileId) ?: fileName.ifBlank { titleFromMessage(message) }
        }
        val content = when (state) {
            "requesting" -> if (direction == "send") "Sending file" else "Downloading file"
            "receiving" -> "Downloading · $percent%"
            "sending" -> if (done == 0L) "Sending file" else "Sending · $percent%"
            "saving" -> "Saving file…"
            "complete" -> if (direction == "send") "Sending finished" else "Download finished"
            "failed" -> "Download failed"
            "expired" -> if (direction == "send") "File offer expired" else "Download expired"
            else -> message
        }

        val builder = NotificationCompat.Builder(context, ClipxCoreForegroundService.TRANSFER_CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_notification_clipx)
            .setContentTitle(title)
            .setContentText(content)
            .setOnlyAlertOnce(true)
            .setOngoing(active)
            .setAutoCancel(!active)

        if (state == "requesting" || state == "saving") {
            builder.setProgress(0, 0, true)
        } else if (total > 0L) {
            builder.setProgress(100, percent, false)
        }

        // A finished, successfully-saved download gets an "Open" action (and
        // becomes the notification's tap target) that hands the file to
        // whatever viewer the OS resolves for it — an image opens in a
        // gallery/viewer, a PDF in a reader, and so on — instead of just
        // bouncing back into Clipx.
        val downloadedFileUri = if (state == "complete" && direction == "download") openableUriFor(message) else null

        if (downloadedFileUri != null) {
            val mimeType = mimeTypeFor(fileName.ifBlank { title })
            val viewIntent = Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(downloadedFileUri, mimeType)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            }
            val openPendingIntent = PendingIntent.getActivity(
                context,
                notificationId(fileId),
                viewIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            builder.setContentIntent(openPendingIntent)
            builder.addAction(R.drawable.ic_notification_clipx, "Open", openPendingIntent)
        } else {
            val openAppIntent = PendingIntent.getActivity(
                context,
                notificationId(fileId),
                Intent(context, MainActivity::class.java).apply {
                    flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
                },
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            builder.setContentIntent(openAppIntent)
        }

        notificationManager.notify(notificationId(fileId), builder.build())
    }
}
