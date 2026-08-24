package com.godiegh.clipx.ui.theme

import android.app.Activity
import android.os.Build
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.BlurredEdgeTreatment
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
private val DarkColorScheme = darkColorScheme(
    primary = ClipxBlue,
    onPrimary = Color(0xFFFFFFFF),
    secondary = ClipxGradientEnd,
    tertiary = ClipxGradientStart,
    background = DarkBackground,
    onBackground = DarkOnBackground,
    surface = DarkSurface,
    onSurface = DarkOnBackground,
    surfaceVariant = DarkCard,
    outline = DarkBorder,
    error = ClipxError,
    onError = Color(0xFFFFFFFF)
)

private val LightColorScheme = lightColorScheme(
    primary = ClipxBlue,
    onPrimary = Color(0xFFFFFFFF),
    secondary = ClipxGradientEnd,
    tertiary = ClipxGradientStart,
    background = LightBackground,
    onBackground = LightOnBackground,
    surface = LightSurface,
    onSurface = LightOnBackground,
    surfaceVariant = LightCard,
    outline = LightBorder,
    error = ClipxError,
    onError = Color(0xFFFFFFFF)
)

@Composable
fun ClipxTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    // Off by default: ClipX has a fixed brand palette (blue/violet) shared
    // with the desktop tray app, so wallpaper-driven Material You colors
    // would break brand consistency across devices. Flip true if you want
    // per-device theming instead.
    dynamicColor: Boolean = false,
    content: @Composable () -> Unit
) {
    val colorScheme = when {
        dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
            val context = LocalContext.current
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        }
        darkTheme -> DarkColorScheme
        else -> LightColorScheme
    }

    val view = LocalView.current
    if (!view.isInEditMode) {
        SideEffect {
            val window = (view.context as Activity).window
            window.statusBarColor = Color.Transparent.toArgb()
            window.navigationBarColor = Color.Transparent.toArgb()
            WindowCompat.getInsetsController(window, view).isAppearanceLightStatusBars = !darkTheme
        }
    }

    MaterialTheme(
        colorScheme = colorScheme,
        typography = Typography,
        shapes = ClipxShapes,
        content = content
    )
}

@Composable
fun ClipxLiveBackground(
    modifier: Modifier = Modifier,
    darkTheme: Boolean = isSystemInDarkTheme(),
) {
    if (!darkTheme) {
        Box(modifier = modifier.fillMaxSize().background(LightBackground))
        return
    }

    val infiniteTransition = rememberInfiniteTransition(label = "spot")

    val spot1X by infiniteTransition.animateFloat(
        initialValue = 0.10f, targetValue = 0.90f,
        animationSpec = infiniteRepeatable(
            animation = tween(14000, easing = FastOutSlowInEasing),
            repeatMode = RepeatMode.Reverse
        ), label = "spot1X"
    )
    val spot1Y by infiniteTransition.animateFloat(
        initialValue = 0.80f, targetValue = 0.15f,
        animationSpec = infiniteRepeatable(
            animation = tween(19000, easing = FastOutSlowInEasing),
            repeatMode = RepeatMode.Reverse
        ), label = "spot1Y"
    )

    val spot2X by infiniteTransition.animateFloat(
        initialValue = 0.85f, targetValue = 0.15f,
        animationSpec = infiniteRepeatable(
            animation = tween(22000, easing = FastOutSlowInEasing),
            repeatMode = RepeatMode.Reverse
        ), label = "spot2X"
    )
    val spot2Y by infiniteTransition.animateFloat(
        initialValue = 0.20f, targetValue = 0.85f,
        animationSpec = infiniteRepeatable(
            animation = tween(16000, easing = FastOutSlowInEasing),
            repeatMode = RepeatMode.Reverse
        ), label = "spot2Y"
    )

    Box(
        modifier = modifier
            .fillMaxSize()
            // Make sure DeepSpaceBackground is exactly Color(0xFF0E1117)
            .background(DeepSpaceBackground)
            .drawBehind {
                val center1 = Offset(size.width * spot1X, size.height * spot1Y)
                val center2 = Offset(size.width * spot2X, size.height * spot2Y)

                // Drastically reduced alpha for a much deeper, moodier glow
                drawRect(
                    brush = Brush.radialGradient(
                        colors = listOf(ClipxGradientStart.copy(alpha = 0.12f), Color.Transparent),
                        center = center1,
                        radius = size.minDimension * 0.8f // Expanded radius for softer falloff
                    )
                )
                drawRect(
                    brush = Brush.radialGradient(
                        colors = listOf(ClipxGradientEnd.copy(alpha = 0.08f), Color.Transparent),
                        center = center2,
                        radius = size.minDimension * 0.7f
                    )
                )
            }
            // Increased blur radius to completely diffuse the light into the #0E1117 background
            .blur(radius = 80.dp, edgeTreatment = BlurredEdgeTreatment.Unbounded)
            .drawWithContent {
                drawContent()

                // Removed the milky white surface tint entirely to preserve true blacks.
                // Left only a microscopic vertical gradient to simulate a tiny bit of screen glare.
                drawRect(
                    brush = Brush.verticalGradient(
                        colors = listOf(
                            Color.White.copy(alpha = 0.02f),
                            Color.Transparent
                        )
                    )
                )
            }
    )
}
