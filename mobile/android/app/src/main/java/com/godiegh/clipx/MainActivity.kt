package com.godiegh.clipx

import android.os.Bundle
import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import com.godiegh.clipx.ui.ClipxApp
import com.godiegh.clipx.ui.theme.ClipxTheme

/**
 * Application entry point for the Android UI.
 *
 * This activity boots the Jetpack Compose shell, requests notification
 * permissions when required, handles shared-text/file intents, and starts the
 * background Clipx service once the user returns to the app.
 */
class MainActivity : ComponentActivity() {
    private val coreViewModel: ClipxCoreViewModel by viewModels {
        object : ViewModelProvider.Factory {
            @Suppress("UNCHECKED_CAST")
            override fun <T : ViewModel> create(modelClass: Class<T>): T =
                ClipxCoreViewModel(application as ClipxApplication) as T
        }
    }

    /**
     * Initializes the activity, configures edge-to-edge UI, and prepares the
     * app for share-intent payloads.
     */
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val wanted = buildList {
            if (Build.VERSION.SDK_INT >= 33) {
                add(Manifest.permission.POST_NOTIFICATIONS)
                add(Manifest.permission.NEARBY_WIFI_DEVICES)
            } else {
                add(Manifest.permission.ACCESS_FINE_LOCATION)
            }
        }.filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
        if (wanted.isNotEmpty()) requestPermissions(wanted.toTypedArray(), 100)
        enableEdgeToEdge()
        handleShareIntent(intent)
        setContent {
            ClipxTheme {
                ClipxApp(coreViewModel)
            }
        }
    }

    override fun onPostResume() {
        super.onPostResume()

        /*
         * The UI needs the Core while Clipx is open.
         *
         * start() deliberately does NOT change Background Sync preference.
         * Therefore, opening Clipx cannot silently turn the user's Quick
         * Settings choice back ON.
         */
        ClipxCoreForegroundService.start(this)
    }

    @Deprecated("This method has been deprecated in favor of using the Activity Result API\n" +
            "      which brings increased type safety via an {@link ActivityResultContract} and the prebuilt\n" +
            "      contracts for common intents available in\n" +
            "      {@link androidx.activity.result.contract.ActivityResultContracts}, provides hooks for\n" +
            "      testing, and allow receiving results in separate, testable classes independent from your\n" +
            "      activity. Use\n" +
            "      {@link #registerForActivityResult(ActivityResultContract, ActivityResultCallback)} passing\n" +
            "      in a {@link RequestMultiplePermissions} object for the {@link ActivityResultContract} and\n" +
            "      handling the result in the {@link ActivityResultCallback#onActivityResult(Object) callback}.")
    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        // Lets the service start Wi-Fi Direct now that the permission may exist.
        ClipxCoreForegroundService.start(this)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) {
            (application as ClipxApplication).onActiveClipboardCheck?.invoke()
        }
    }

    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleShareIntent(intent)
    }

    /**
     * Consumes Android share intents and forwards the payload into the core view model.
     */
    private fun handleShareIntent(intent: android.content.Intent?) {
        if (intent?.action != android.content.Intent.ACTION_SEND && intent?.action != android.content.Intent.ACTION_SEND_MULTIPLE) return
        val uris = buildList {
            intent.getParcelableExtra<android.net.Uri>(android.content.Intent.EXTRA_STREAM)?.let(::add)
            intent.getParcelableArrayListExtra<android.net.Uri>(android.content.Intent.EXTRA_STREAM)
                ?.let { addAll(it) }
            intent.clipData?.let { clip -> for (i in 0 until clip.itemCount) clip.getItemAt(i).uri?.let(::add) }
        }.distinct()
        if (uris.isNotEmpty()) coreViewModel.enqueueFiles(this, uris)
        else intent.getStringExtra(android.content.Intent.EXTRA_TEXT)?.let(coreViewModel::enqueueSharedText)
        if (uris.isNotEmpty()) coreViewModel.openSharedSync()
    }
}
