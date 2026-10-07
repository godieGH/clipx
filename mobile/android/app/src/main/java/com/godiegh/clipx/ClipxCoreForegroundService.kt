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
import androidx.core.content.ContextCompat
import androidx.core.content.edit

class ClipxCoreForegroundService : Service() {
    companion object {
        private const val CHANNEL_ID = "clipx_core"
        private const val NOTIFICATION_ID = 1001

        const val PROMPT_CHANNEL_ID = "clipx_prompts"
        const val TRANSFER_CHANNEL_ID = "clipx_transfers"

        private const val ACTION_KILL_CLIPX =
            "com.godiegh.clipx.action.KILL_PROCESS"

        /*
         * User preference:
         * whether Clipx should keep its Core running in the background.
         *
         * Keep the same preference file/key used by the first implementation
         * so an existing installed debug build keeps its current setting.
         */
        private const val CORE_PREFS = "clipx_core_state"
        private const val KEY_BACKGROUND_SYNC_ENABLED = "enabled"

        fun isBackgroundSyncEnabled(
            context: android.content.Context,
        ): Boolean =
            context.getSharedPreferences(
                CORE_PREFS,
                android.content.Context.MODE_PRIVATE,
            ).getBoolean(
                KEY_BACKGROUND_SYNC_ENABLED,
                true,
            )

        private fun setBackgroundSyncEnabledPreference(
            context: android.content.Context,
            enabled: Boolean,
        ) {
            context.getSharedPreferences(
                CORE_PREFS,
                android.content.Context.MODE_PRIVATE,
            )
                .edit {
                    putBoolean(
                        KEY_BACKGROUND_SYNC_ENABLED,
                        enabled,
                    )
                }
        }

        private fun requestQuickSettingsTileUpdate(
            context: android.content.Context,
        ) {
            android.service.quicksettings.TileService.requestListeningState(
                context,
                android.content.ComponentName(
                    context,
                    ClipxQuickSettingsTile::class.java,
                ),
            )
        }

        /**
         * Starts the Core without changing the user's Background Sync setting.
         *
         * Used when:
         * - Clipx UI needs the Core
         * - refresh/restart needs the Core
         */
        fun start(context: android.content.Context) {
            val intent = Intent(
                context,
                ClipxCoreForegroundService::class.java,
            )

            ContextCompat.startForegroundService(context, intent)
        }

        /**
         * Enables Background Sync and starts the Core.
         */
        fun enableBackgroundSync(
            context: android.content.Context,
        ) {
            setBackgroundSyncEnabledPreference(
                context,
                true,
            )

            try {
                start(context)
            } catch (t: Throwable) {
                // If Android rejects the foreground-service start immediately,
                // don't leave the preference saying that Background Sync is ON.
                setBackgroundSyncEnabledPreference(
                    context,
                    false,
                )
                throw t
            }

            requestQuickSettingsTileUpdate(context)
        }

        /**
         * Disables Background Sync.
         *
         * If the Clipx UI is still visible, the Core remains running because
         * the UI still needs the BridgeService.
         *
         * If the UI is not visible, the Core is stopped.
         */
        fun disableBackgroundSync(
            context: android.content.Context,
        ) {
            setBackgroundSyncEnabledPreference(
                context,
                false,
            )

            requestQuickSettingsTileUpdate(context)
            stopIfNotNeeded(context)
        }

        /**
         * Stops the Core only when:
         * - Background Sync is disabled, and
         * - no Clipx Activity is visible.
         */
        fun stopIfNotNeeded(
            context: android.content.Context,
        ) {
            val app =
                context.applicationContext as ClipxApplication

            if (
                !isBackgroundSyncEnabled(context) &&
                !app.isActivityVisible
            ) {
                app.setBridgeService(null)

                context.stopService(
                    Intent(
                        context,
                        ClipxCoreForegroundService::class.java,
                    ),
                )
            }
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
    private var wifiDirect: WifiDirectLink? = null

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
            disableBackgroundSync(this)
            stopCoreAndSelf(wait = true)
            stopSelfResult(startId)
            Process.killProcess(Process.myPid())
            return START_NOT_STICKY
        }
        if (!listening) {
            clipboardPlatform.startListening()
            listening = true
        }

        if (coreStarted) {
            // Re-entry (e.g. after Wi-Fi Direct permission was just granted).
            wifiDirect?.start()
            return START_STICKY
        }
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
            runCatching { bridgeService.getIdentity() }.onSuccess { id ->
                wifiDirect = WifiDirectLink(
                    applicationContext, id.fingerprint, id.name, id.wsPort.toInt(),
                    hasPeers = {
                        runCatching {
                            bridgeService.getAvailableDevices().isNotEmpty() ||
                                bridgeService.getPairedDevices().any { it.connection != "unavailable" }
                        }.getOrDefault(true)
                    },
                ).also { it.start() }
            }
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
        wifiDirect?.stop()
        wifiDirect = null
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
