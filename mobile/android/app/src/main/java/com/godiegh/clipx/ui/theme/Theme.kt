package com.godiegh.clipx.ui.theme

import android.app.Activity
import android.os.Build
import androidx.compose.animation.core.FastOutSlowInEasing
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
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat

val LocalClipxPalette = staticCompositionLocalOf { DarkClipxPalette }

private val DarkColorScheme = darkColorScheme(
    primary = ClipxBlue,
    onPrimary = Color.White,
    secondary = ClipxPurple,
    onSecondary = Color.White,
    tertiary = ClipxGradientEnd,
    background = DarkClipxPalette.background,
    onBackground = DarkOnBackground,
    surface = DarkClipxPalette.surface,
    onSurface = DarkOnBackground,
    surfaceVariant = DarkClipxPalette.card,
    onSurfaceVariant = DarkClipxPalette.mutedText,
    outline = DarkClipxPalette.cardBorder,
    error = ClipxError,
    onError = Color.White,
)

private val LightColorScheme = lightColorScheme(
    primary = ClipxBlue,
    onPrimary = Color.White,
    secondary = ClipxPurple,
    onSecondary = Color.White,
    tertiary = ClipxGradientEnd,
    background = LightClipxPalette.background,
    onBackground = LightOnBackground,
    surface = LightClipxPalette.surface,
    onSurface = LightOnBackground,
    surfaceVariant = LightClipxPalette.card,
    onSurfaceVariant = LightClipxPalette.mutedText,
    outline = LightClipxPalette.cardBorder,
    error = ClipxError,
    onError = Color.White,
)

@Composable
fun ClipxTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    dynamicColor: Boolean = false,
    content: @Composable () -> Unit,
) {
    val palette = if (darkTheme) DarkClipxPalette else LightClipxPalette
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
            val controller = WindowCompat.getInsetsController(window, view)
            controller.isAppearanceLightStatusBars = !darkTheme
            controller.isAppearanceLightNavigationBars = !darkTheme
        }
    }

    CompositionLocalProvider(LocalClipxPalette provides palette) {
        MaterialTheme(
            colorScheme = colorScheme,
            typography = Typography,
            shapes = ClipxShapes,
            content = content,
        )
    }
}

@Composable
fun ClipxLiveBackground(
    modifier: Modifier = Modifier,
    darkTheme: Boolean = isSystemInDarkTheme(),
) {
    if (!darkTheme) {
        Box(modifier = modifier.fillMaxSize().background(LightClipxPalette.background))
        return
    }

    val transition = rememberInfiniteTransition(label = "clipxBackground")
    val x1 by transition.animateFloat(
        initialValue = .08f,
        targetValue = .82f,
        animationSpec = infiniteRepeatable(tween(16000, easing = FastOutSlowInEasing), RepeatMode.Reverse),
        label = "x1",
    )
    val y1 by transition.animateFloat(
        initialValue = .80f,
        targetValue = .16f,
        animationSpec = infiniteRepeatable(tween(19000, easing = FastOutSlowInEasing), RepeatMode.Reverse),
        label = "y1",
    )
    val x2 by transition.animateFloat(
        initialValue = .90f,
        targetValue = .18f,
        animationSpec = infiniteRepeatable(tween(22000, easing = FastOutSlowInEasing), RepeatMode.Reverse),
        label = "x2",
    )
    val y2 by transition.animateFloat(
        initialValue = .18f,
        targetValue = .82f,
        animationSpec = infiniteRepeatable(tween(17000, easing = FastOutSlowInEasing), RepeatMode.Reverse),
        label = "y2",
    )

    Box(
        modifier = modifier
            .fillMaxSize()
            .background(DeepSpaceBackground)
            .drawBehind {
                drawRect(
                    brush = Brush.radialGradient(
                        colors = listOf(ClipxGradientStart.copy(alpha = .11f), Color.Transparent),
                        center = Offset(size.width * x1, size.height * y1),
                        radius = size.minDimension * .78f,
                    )
                )
                drawRect(
                    brush = Brush.radialGradient(
                        colors = listOf(ClipxBlue.copy(alpha = .075f), Color.Transparent),
                        center = Offset(size.width * x2, size.height * y2),
                        radius = size.minDimension * .70f,
                    )
                )
            },
    )
}
