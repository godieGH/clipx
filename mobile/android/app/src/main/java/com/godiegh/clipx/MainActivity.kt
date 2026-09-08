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

class MainActivity : ComponentActivity() {
    private val coreViewModel: ClipxCoreViewModel by viewModels {
        object : ViewModelProvider.Factory {
            @Suppress("UNCHECKED_CAST")
            override fun <T : ViewModel> create(modelClass: Class<T>): T =
                ClipxCoreViewModel(application as ClipxApplication) as T
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 100)
        }
        enableEdgeToEdge()
        handleShareIntent(intent)
        setContent {
            ClipxTheme {
                ClipxApp(coreViewModel)
            }
        }
    }

    private var serviceStartRequested = false

    override fun onPostResume() {
        super.onPostResume()
        if (!serviceStartRequested) {
            serviceStartRequested = true
            ClipxCoreForegroundService.start(this)
        }
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

    private fun handleShareIntent(intent: android.content.Intent?) {
        if (intent?.action != android.content.Intent.ACTION_SEND && intent?.action != android.content.Intent.ACTION_SEND_MULTIPLE) return
        val uris = buildList {
            intent?.getParcelableExtra<android.net.Uri>(android.content.Intent.EXTRA_STREAM)?.let(::add)
            intent?.getParcelableArrayListExtra<android.net.Uri>(android.content.Intent.EXTRA_STREAM)?.let { addAll(it) }
            intent?.clipData?.let { clip -> for (i in 0 until clip.itemCount) clip.getItemAt(i).uri?.let(::add) }
        }.distinct()
        if (uris.isNotEmpty()) coreViewModel.enqueueFiles(this, uris)
        else intent?.getStringExtra(android.content.Intent.EXTRA_TEXT)?.let(coreViewModel::enqueueSharedText)
        if (uris.isNotEmpty()) coreViewModel.openSharedSync()
    }
}
