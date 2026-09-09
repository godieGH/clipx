package com.godiegh.clipx

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.os.Build
import android.os.Process
import android.os.IBinder
import android.provider.Settings
import androidx.core.app.NotificationCompat
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
        const val TRANSFER_CHANNEL_ID = "clipx_transfers"
        private const val ACTION_KILL_CLIPX = "com.godiegh.clipx.action.KILL_PROCESS"

        fun start(context: android.content.Context) {
            val intent = Intent(context, ClipxCoreForegroundService::class.java)
            context.startService(intent)
        }
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private lateinit var bridgeService: BridgeService
    private lateinit var clipboardPlatform: AndroidClipboardPlatform
    private lateinit var notificationPlatform: AndroidNotificationPlatform
    private lateinit var eventListener: AndroidCoreEventListener
    private var listening = false
    private var coreStarted = false
    private var cleanedUp = false

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
        if (intent?.action == ACTION_KILL_CLIPX) {
            stopCoreAndSelf(wait = true)
            stopSelfResult(startId)
            Process.killProcess(Process.myPid())
            return START_NOT_STICKY
        }
        if (!listening) {
            clipboardPlatform.startListening()
            listening = true
        }

        if (coreStarted) return START_STICKY
        coreStarted = true

        scope.launch {
            bridgeService.start(
                dataDir = noBackupFilesDir.absolutePath,
                deviceName = resolveDeviceName(),
                clipboard = clipboardPlatform,
                notifier = notificationPlatform,
                event = eventListener,
            )
            // Only now are the Rust command channels actually wired. If the
            // Activity resumed while the service was starting, its lifecycle
            // callback may already have fired; re-check after the core is
            // ready so that clipboard changes made in another app are not lost.
            val app = application as ClipxApplication
            app.setBridgeService(bridgeService)
            if (app.isActivityVisible && AndroidClipboardPlatform.isAutoSyncOnResumeEnabled(this@ClipxCoreForegroundService)) {
                clipboardPlatform.checkClipboardNow()
            }
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
        coreStarted = false
        listening = false
        stopCoreAndSelf(wait = true)
        scope.cancel()
        super.onDestroy()
    }

    private fun clearBridgeIfOwned(app: ClipxApplication) {
        if (app.bridgeService === bridgeService) app.setBridgeService(null)
    }

    private fun stopCoreAndSelf(wait: Boolean = false) {
        if (cleanedUp) return
        cleanedUp = true
        if (listening) {
            clipboardPlatform.stopListening()
            listening = false
        }

        (application as ClipxApplication).onActiveClipboardCheck = null

        if (wait) {
            if (::bridgeService.isInitialized) runBlocking { bridgeService.stop() }
            clearBridgeIfOwned(application as ClipxApplication)
            return
        }

        scope.launch {
            if (::bridgeService.isInitialized) bridgeService.stop()
            clearBridgeIfOwned(application as ClipxApplication)
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
        val killIntent = Intent(this, ClipxCoreForegroundService::class.java).apply { action = ACTION_KILL_CLIPX }
        val killPendingIntent = PendingIntent.getService(
            this,
            1002,
            killIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_notification_clipx)
            .setContentTitle("Clipx")
            .setContentText("Clipboard synchronization is running")
            .setContentIntent(pendingIntent)
            .addAction(R.drawable.ic_notification_clipx, "Kill Clipx", killPendingIntent)
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
        manager.createNotificationChannel(
            NotificationChannel(
                TRANSFER_CHANNEL_ID,
                "Clipx transfers",
                NotificationManager.IMPORTANCE_LOW,
            ).apply { setShowBadge(true) },
        )
    }

    private fun resolveDeviceName(): String {
        val settingsName = Settings.Global.getString(contentResolver, Settings.Global.DEVICE_NAME)
        return if (!settingsName.isNullOrBlank()) settingsName else Build.MODEL
    }
}
