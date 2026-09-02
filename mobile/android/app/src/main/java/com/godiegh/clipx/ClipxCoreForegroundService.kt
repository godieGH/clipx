package com.godiegh.clipx

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import com.godiegh.clipx.ffi.BridgeService
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

class ClipxCoreForegroundService : Service() {
    companion object {
        private const val CHANNEL_ID = "clipx_core"
        private const val NOTIFICATION_ID = 1001

        fun start(context: android.content.Context) {
            val intent = Intent(context, ClipxCoreForegroundService::class.java)
            ContextCompat.startForegroundService(context, intent)
        }
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private lateinit var bridgeService: BridgeService
    private lateinit var clipboardPlatform: AndroidClipboardPlatform
    private lateinit var notificationPlatform: AndroidNotificationPlatform
    private lateinit var eventListener: AndroidCoreEventListener
    private var listening = false

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
        startForeground(NOTIFICATION_ID, buildNotification())

        bridgeService = BridgeService()
        clipboardPlatform = AndroidClipboardPlatform(this, bridgeService)
        notificationPlatform = AndroidNotificationPlatform(this, bridgeService)
        eventListener = AndroidCoreEventListener()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (!listening) {
            clipboardPlatform.startListening()
            listening = true
        }

        scope.launch {
            bridgeService.start(
                dataDir = noBackupFilesDir.absolutePath,
                clipboard = clipboardPlatform,
                notifier = notificationPlatform,
                event = eventListener,
            )
        }

        return START_STICKY
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        stopCoreAndSelf()
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        stopCoreAndSelf(wait = true)
        scope.cancel()
        super.onDestroy()
    }

    private fun stopCoreAndSelf(wait: Boolean = false) {
        if (listening) {
            clipboardPlatform.stopListening()
            listening = false
        }

        if (wait) {
            runBlocking { bridgeService.stop() }
            return
        }

        scope.launch {
            bridgeService.stop()
            stopSelf()
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun buildNotification(): Notification {
        val pendingIntent = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle("Clipx")
            .setContentText("Clipboard synchronization is running")
            .setContentIntent(pendingIntent)
            .setOngoing(true)
            .build()
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val manager = getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(
                NotificationChannel(
                    CHANNEL_ID,
                    "Clipx core service",
                    NotificationManager.IMPORTANCE_LOW,
                ),
            )
        }
    }
}
