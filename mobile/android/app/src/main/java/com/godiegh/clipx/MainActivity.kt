package com.godiegh.clipx

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import com.godiegh.clipx.ui.ClipxApp
import com.godiegh.clipx.ui.theme.ClipxTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            ClipxTheme {
                ClipxApp()
            }
        }
    }
}
