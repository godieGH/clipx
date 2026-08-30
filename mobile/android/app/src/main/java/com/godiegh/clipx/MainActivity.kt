package com.godiegh.clipx

import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.lifecycle.lifecycleScope
import com.godiegh.clipx.ui.ClipxApp
import com.godiegh.clipx.ui.theme.ClipxTheme
import kotlinx.coroutines.async
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        uniffi.rust_android_ffi_bridge.initLogging()
        setContent {
            ClipxTheme {
                ClipxApp()
            }
        }
    }
}