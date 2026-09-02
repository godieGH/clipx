package com.godiegh.clipx

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.os.Build
import android.os.IBinder
import android.provider.Settings
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
        const val PROMPT_CHANNEL_ID = "clipx_prompts"

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
        eventListener = AndroidCoreEventListener(application as ClipxApplication)
        (application as ClipxApplication).onActiveClipboardCheck = { clipboardPlatform.checkClipboardNow() }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (!listening) {
            clipboardPlatform.startListening()
            listening = true
        }

        scope.launch {
            bridgeService.start(
                dataDir = noBackupFilesDir.absolutePath,
                deviceName = resolveDeviceName(),
                clipboard = clipboardPlatform,
                notifier = notificationPlatform,
                event = eventListener,
            )
            // Only now is device_tx/clipboard_cmd_tx actually wired up on the
            // Rust side — publishing any earlier lets the ViewModel's
            // awaitBridgeService() unblock and call getIdentity() before the
            // core can answer it, so it silently fails and identity stays
            // "Unknown" until something else forces a re-fetch.
            (application as ClipxApplication).setBridgeService(bridgeService)
        }

        return START_STICKY
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        // intentionally not stopping — core keeps running after the task
        // is swiped; user stops it
        // stopCoreAndSelf()
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

        (application as ClipxApplication).onActiveClipboardCheck = null

        if (wait) {
            runBlocking { bridgeService.stop() }
            (application as ClipxApplication).setBridgeService(null)
            return
        }

        scope.launch {
            bridgeService.stop()
            (application as ClipxApplication).setBridgeService(null)
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
            .setSmallIcon(R.drawable.ic_notification_clipx)
            .setContentTitle("Clipx")
            .setContentText("Clipboard synchronization is running")
            .setContentIntent(pendingIntent)
            .setOngoing(true)
            .build()
    }

    private fun createNotificationChannel() {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                "Clipx core service",
                NotificationManager.IMPORTANCE_LOW,
            ).apply { setShowBadge(false) },
        )
        manager.createNotificationChannel(
            NotificationChannel(
                PROMPT_CHANNEL_ID,
                "Clipx requests",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply { setShowBadge(true) },
        )
    }

    private fun resolveDeviceName(): String {
        val settingsName = Settings.Global.getString(contentResolver, Settings.Global.DEVICE_NAME)
        return if (!settingsName.isNullOrBlank()) settingsName else Build.MODEL
    }
}
