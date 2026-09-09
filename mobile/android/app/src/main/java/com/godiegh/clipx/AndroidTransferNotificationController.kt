package com.godiegh.clipx

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
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

    fun update(fileId: String, done: Long, total: Long, state: String, message: String) {
        val percent = if (total > 0L) ((done.toDouble() / total.toDouble()) * 100.0).toInt().coerceIn(0, 100) else 0
        val active = state == "requesting" || state == "receiving" || state == "sending" || state == "saving"
        val title = if (active) {
            val resolved = titleFromMessage(message)
            titles.putIfAbsent(fileId, resolved) ?: resolved
        } else {
            titles.remove(fileId) ?: titleFromMessage(message)
        }
        val content = when (state) {
            "requesting" -> "Starting download…"
            "receiving" -> "Downloading · $percent%"
            "sending" -> "Sending · $percent%"
            "saving" -> "Saving file…"
            "complete" -> "Downloaded successfully"
            "failed" -> "Download failed"
            "expired" -> "Download expired"
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

        val openIntent = PendingIntent.getActivity(
            context,
            notificationId(fileId),
            Intent(context, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
            },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        builder.setContentIntent(openIntent)

        notificationManager.notify(notificationId(fileId), builder.build())
    }
}
