package com.godiegh.clipx.ui.theme

import androidx.compose.ui.graphics.Color

// Brand tokens. Keep the exact documented ClipX blue from the desktop app.
val ClipxBlue = Color(0xFF396CD8)
val ClipxBlueHover = Color(0xFF2F5BC0)
val ClipxBluePressed = Color(0xFF264B9E)
val ClipxPurple = Color(0xFF7B61FF)
val ClipxGradientStart = Color(0xFF1D0BB7)
val ClipxGradientEnd = Color(0xFF6647E6)

val ClipxError = Color(0xFFFF5357)
val ClipxErrorStrong = Color(0xFFFF4B50)
val ClipxSuccess = Color(0xFF43D17A)
val ClipxWarning = Color(0xFFF1B84B)

/** UI-only semantic palette. It changes with the system theme. */
data class ClipxPalette(
    val background: Color,
    val surface: Color,
    val card: Color,
    val cardBorder: Color,
    val mutedText: Color,
    val subtleText: Color,
    val divider: Color,
    val avatar: Color,
    val avatarBorder: Color,
    val historyBadge: Color,
    val textBadge: Color,
    val codeBadge: Color,
    val actionContainer: Color,
    val navigationSurface: Color,
    val sectionLabel: Color,
)

val DarkClipxPalette = ClipxPalette(
    background = Color(0xFF0E1117),
    surface = Color(0xFF09111B),
    card = Color(0xB20F1925),
    cardBorder = Color(0x2637485D),
    mutedText = Color(0xFF9AA6B8),
    subtleText = Color(0xFF7F8B9D),
    divider = Color(0x2237485D),
    avatar = Color(0xFF182438),
    avatarBorder = Color(0x2437485D),
    historyBadge = Color(0xFF101A27),
    textBadge = Color(0xFF62DF8B),
    codeBadge = Color(0xFFFFB11B),
    actionContainer = Color(0x26396CD8),
    navigationSurface = Color(0xF509111B),
    sectionLabel = ClipxPurple,
)

// Light surfaces deliberately use soft neutrals instead of dark translucent cards.
val LightClipxPalette = ClipxPalette(
    background = Color(0xFFF5F7FB),
    surface = Color(0xFFFFFFFF),
    card = Color(0xFFECEFF4),
    cardBorder = Color(0xFFDDE2EA),
    mutedText = Color(0xFF647081),
    subtleText = Color(0xFF808B99),
    divider = Color(0xFFDDE2EA),
    avatar = Color(0xFFE2E7EF),
    avatarBorder = Color(0xFFD4DAE3),
    historyBadge = Color(0xFFE1E6ED),
    textBadge = Color(0xFF1F8D5A),
    codeBadge = Color(0xFFB26C00),
    actionContainer = Color(0x14396CD8),
    navigationSurface = Color(0xF5FFFFFF),
    sectionLabel = Color(0xFF5F6875),
)

val DeepSpaceBackground = DarkClipxPalette.background
val DarkBackground = DarkClipxPalette.background
val DarkOnBackground = Color(0xFFF3F6FC)
val DarkSurface = DarkClipxPalette.surface

val LightBackground = LightClipxPalette.background
val LightOnBackground = Color(0xFF151A22)
val LightSurface = LightClipxPalette.surface

// Compatibility aliases used by older UI code. New code should use LocalClipxPalette.current.
val ClipxBackground = DarkBackground
val ClipxSurface = DarkSurface
val ClipxCard = DarkClipxPalette.card
val ClipxCardBorder = DarkClipxPalette.cardBorder
val ClipxMutedText = DarkClipxPalette.mutedText
val ClipxSubtleText = DarkClipxPalette.subtleText
val ClipxHistoryBadge = DarkClipxPalette.historyBadge
val ClipxTextBadge = DarkClipxPalette.textBadge
val ClipxCode = DarkClipxPalette.codeBadge
val ClipxAvatarDark = DarkClipxPalette.avatar
val ClipxAvatarDarker = Color(0xFF111A27)
