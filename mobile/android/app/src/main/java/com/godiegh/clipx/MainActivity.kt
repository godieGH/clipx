package com.godiegh.clipx

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Scaffold
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import com.godiegh.clipx.ui.theme.ClipxTheme
import com.godiegh.clipx.ui.theme.ClipxLiveBackground

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

@Composable
fun ClipxApp() {
    // this is where our app starts
    Box(modifier = Modifier.fillMaxSize()) {
        ClipxLiveBackground(modifier = Modifier.fillMaxSize())
        Scaffold(containerColor = Color.Transparent) { innerPadding ->
            // our app content goes here
        }
    }

}

