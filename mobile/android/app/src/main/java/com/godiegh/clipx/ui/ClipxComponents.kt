package com.godiegh.clipx.ui

import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Android
import androidx.compose.material.icons.filled.AttachFile
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.DeleteOutline
import androidx.compose.material.icons.filled.DesktopWindows
import androidx.compose.material.icons.filled.DevicesOther
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.Laptop
import androidx.compose.material.icons.filled.Link
import androidx.compose.material.icons.filled.PhoneAndroid
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Terminal
import androidx.compose.material.icons.filled.TextFields
import androidx.compose.material.icons.filled.Wifi
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Divider
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.ViewModel
import com.godiegh.clipx.ClipType
import com.godiegh.clipx.ConnectionStatus
import com.godiegh.clipx.DeviceType
import com.godiegh.clipx.PairedDevice
import com.godiegh.clipx.R
import com.godiegh.clipx.ui.theme.ClipxBlue
import com.godiegh.clipx.ui.theme.ClipxError
import com.godiegh.clipx.ui.theme.ClipxGradientEnd
import com.godiegh.clipx.ui.theme.ClipxGradientStart
import com.godiegh.clipx.ui.theme.LocalClipxPalette
val LocalClipxSheetController = staticCompositionLocalOf<ClipxSheetController> {
    error("Clipx sheet controller is not provided")
}

@Composable
fun ClipxLogo(modifier: Modifier = Modifier, size: Dp = 34.dp) {
    Image(
        painter = painterResource(id = R.drawable.ic_launcher_foreground),
        contentDescription = "Clipx",
        contentScale = ContentScale.Crop,
        modifier = modifier
            .size(size)
            .clip(RoundedCornerShape(size / 3)),
    )
}

@Composable
fun AppTopBar(
    title: String,
    showLogo: Boolean = false,
    onBack: (() -> Unit)? = null,
    rightContent: @Composable RowScope.() -> Unit = {},
) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 10.dp, vertical = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (onBack != null) {
            IconButton(onClick = onBack, modifier = Modifier.size(42.dp)) {
                Icon(
                    Icons.AutoMirrored.Filled.ArrowBack,
                    contentDescription = "Back",
                    tint = MaterialTheme.colorScheme.onSurface,
                )
            }
            Spacer(Modifier.width(2.dp))
        }
        if (showLogo) {
            ClipxLogo(size = 34.dp)
            Spacer(Modifier.width(9.dp))
        }
        Text(
            text = title,
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Medium,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.weight(1f),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        rightContent()
    }
}

@Composable
fun ScreenHeader(
    title: String,
    onBack: (() -> Unit)? = null,
    rightContent: @Composable RowScope.() -> Unit = {},
) = AppTopBar(title = title, onBack = onBack, rightContent = rightContent)

@Composable
fun ClipxScaffold(
    currentTab: RootTab,
    showBottomBar: Boolean,
    onDevices: () -> Unit,
    onHistory: () -> Unit,
    content: @Composable (Modifier) -> Unit,
) {
    Scaffold(
        containerColor = Color.Transparent,
        bottomBar = {
            if (showBottomBar) {
                ClipxBottomBar(currentTab, onDevices, onHistory)
            }
        },
    ) { innerPadding ->
        content(
            Modifier
                .fillMaxSize()
                .padding(innerPadding),
        )
    }
}

@Composable
private fun ClipxBottomBar(
    currentTab: RootTab,
    onDevices: () -> Unit,
    onHistory: () -> Unit,
) {
    val palette = LocalClipxPalette.current
    Surface(
        modifier = Modifier.fillMaxWidth().windowInsetsPadding(androidx.compose.foundation.layout.WindowInsets.navigationBars),
        color = palette.navigationSurface,
        tonalElevation = 0.dp,
        border = BorderStroke(1.dp, palette.cardBorder),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().height(64.dp),
            horizontalArrangement = Arrangement.SpaceEvenly,
        ) {
            BottomTab(currentTab == RootTab.DEVICES, Icons.Filled.DesktopWindows, "Devices", onDevices)
            BottomTab(currentTab == RootTab.HISTORY, Icons.Filled.ContentCopy, "History", onHistory)
        }
    }
}

@Composable
private fun BottomTab(selected: Boolean, icon: ImageVector, label: String, onClick: () -> Unit) {
    val color = if (selected) ClipxBlue else MaterialTheme.colorScheme.onSurfaceVariant
    Column(
        modifier = Modifier.fillMaxHeight().width(120.dp).clickable(onClick = onClick),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(icon, contentDescription = label, tint = color, modifier = Modifier.size(22.dp))
        Text(label, color = color, style = MaterialTheme.typography.labelSmall, modifier = Modifier.padding(top = 3.dp))
    }
}

@Composable
fun SectionLabel(text: String, action: (@Composable () -> Unit)? = null) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text,
            style = MaterialTheme.typography.labelSmall,
            color = LocalClipxPalette.current.sectionLabel,
            modifier = Modifier.weight(1f),
        )
        action?.invoke()
    }
}

@Composable
fun GlassCard(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    val palette = LocalClipxPalette.current
    Column(
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(palette.card)
            .border(BorderStroke(1.dp, palette.cardBorder), RoundedCornerShape(12.dp))
            .padding(10.dp),
        content = content,
    )
}

@Composable
fun DeviceAvatar(type: DeviceType, modifier: Modifier = Modifier, size: Dp = 48.dp) {
    val palette = LocalClipxPalette.current
    val icon = when (type) {
        DeviceType.WINDOWS -> Icons.Filled.DesktopWindows
        DeviceType.MACOS -> Icons.Filled.Laptop
        DeviceType.LINUX -> Icons.Filled.Terminal
        DeviceType.ANDROID -> Icons.Filled.Android
        DeviceType.IOS -> Icons.Filled.PhoneAndroid
        DeviceType.UNKNOWN -> Icons.Filled.DevicesOther
    }
    Box(
        modifier = modifier
            .size(size)
            .clip(RoundedCornerShape(12.dp))
            .background(palette.avatar)
            .border(1.dp, palette.avatarBorder, RoundedCornerShape(12.dp)),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            icon,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.size(size * .48f),
        )
    }
}

@Composable
fun StatusDot(status: ConnectionStatus) {
    val palette = LocalClipxPalette.current
    val color = when (status) {
        ConnectionStatus.CONNECTED -> com.godiegh.clipx.ui.theme.ClipxSuccess
        ConnectionStatus.CONNECTING -> com.godiegh.clipx.ui.theme.ClipxWarning
        ConnectionStatus.DISCONNECTED -> palette.mutedText.copy(alpha = .65f)
        ConnectionStatus.UNAVAILABLE -> palette.mutedText.copy(alpha = .48f)
    }
    Box(Modifier.size(8.dp).clip(CircleShape).background(color))
}

@Composable
fun ActionButton(
    text: String,
    onClick: () -> Unit,
    enabled: Boolean = true,
    modifier: Modifier = Modifier,
) {
    val palette = LocalClipxPalette.current
    Button(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier.defaultMinSize(minWidth = 58.dp, minHeight = 34.dp),
        shape = RoundedCornerShape(10.dp),
        contentPadding = PaddingValues(horizontal = 13.dp, vertical = 0.dp),
        colors = ButtonDefaults.buttonColors(
            containerColor = palette.actionContainer,
            contentColor = MaterialTheme.colorScheme.primary,
            disabledContainerColor = MaterialTheme.colorScheme.onSurface.copy(alpha = .08f),
            disabledContentColor = MaterialTheme.colorScheme.onSurface.copy(alpha = .45f),
        ),
        elevation = null,
    ) {
        Text(text, style = MaterialTheme.typography.labelLarge, maxLines = 1)
    }
}

@Composable
fun DeviceRow(
    device: PairedDevice,
    onClick: () -> Unit,
    actionLabel: String? = null,
    onAction: (() -> Unit)? = null,
) {
    GlassCard(modifier = Modifier.padding(horizontal = 18.dp, vertical = 4.dp)) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.fillMaxWidth().clickable(onClick = onClick),
        ) {
            DeviceAvatar(device.type, size = 46.dp)
            Spacer(Modifier.width(11.dp))
            Column(Modifier.weight(1f).padding(vertical = 2.dp)) {
                Text(
                    device.name,
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 2.dp)) {
                    StatusDot(device.status)
                    Spacer(Modifier.width(7.dp))
                    Text(
                        connectionLabel(device.status),
                        color = LocalClipxPalette.current.mutedText,
                        style = MaterialTheme.typography.bodyMedium,
                        maxLines = 1,
                    )
                }
            }
            if (actionLabel != null && onAction != null) {
                Spacer(Modifier.width(10.dp))
                ActionButton(actionLabel, onAction)
            }
        }
    }
}

@Composable
fun AvailableDeviceRow(
    device: AvailableDeviceUi,
    onPair: () -> Unit,
) {
    GlassCard(modifier = Modifier.padding(horizontal = 18.dp, vertical = 4.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
            DeviceAvatar(device.model.type, size = 46.dp)
            Spacer(Modifier.width(11.dp))
            Column(Modifier.weight(1f).padding(vertical = 2.dp)) {
                Text(
                    device.model.name,
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    formatFingerprint(device.model.id),
                    color = LocalClipxPalette.current.mutedText,
                    style = MaterialTheme.typography.bodyMedium.copy(fontSize = 10.sp, letterSpacing = .05.sp),
                    maxLines = 1,
                    overflow = TextOverflow.Clip,
                )
            }
            Spacer(Modifier.width(8.dp))
            ActionButton(if (device.requesting) "Requesting…" else "Pair", onPair, enabled = !device.requesting)
        }
    }
}

@Composable
fun ScanRadar() {
    val transition = rememberInfiniteTransition(label = "scan")
    val pulse by transition.animateFloat(
        initialValue = .84f,
        targetValue = 1.12f,
        animationSpec = infiniteRepeatable(tween(1800, easing = FastOutSlowInEasing), RepeatMode.Reverse),
        label = "pulse",
    )
    Box(modifier = Modifier.size(220.dp), contentAlignment = Alignment.Center) {
        RadarRing(200.dp, .12f)
        RadarRing((170 * pulse).dp, .14f)
        RadarRing((140 * pulse).dp, .17f)
        Box(
            modifier = Modifier.size(70.dp).clip(CircleShape).background(Brush.linearGradient(listOf(ClipxBlue, ClipxGradientEnd))),
            contentAlignment = Alignment.Center,
        ) {
            Icon(Icons.Filled.Link, contentDescription = null, tint = Color.White, modifier = Modifier.size(36.dp))
        }
    }
}

@Composable
private fun RadarRing(size: Dp, alpha: Float) {
    Box(modifier = Modifier.size(size).clip(CircleShape).border(1.dp, ClipxBlue.copy(alpha = alpha), CircleShape))
}

@Composable
fun InfoPair(label: String, value: String, modifier: Modifier = Modifier) {
    val palette = LocalClipxPalette.current
    Column(modifier) {
        Text(label, style = MaterialTheme.typography.labelSmall, color = palette.mutedText)
        Text(
            value,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurface,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.padding(top = 2.dp),
        )
    }
}

@Composable
fun ToggleRow(label: String, description: String, checked: Boolean, onCheckedChange: (Boolean) -> Unit) {
    val palette = LocalClipxPalette.current
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 4.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(label, color = MaterialTheme.colorScheme.onSurface, style = MaterialTheme.typography.bodyLarge)
            Text(description, color = palette.mutedText, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
        Spacer(Modifier.width(8.dp))
        Switch(checked = checked, onCheckedChange = onCheckedChange)
    }
}

@Composable
fun DangerRow(label: String, icon: ImageVector, onClick: () -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 4.dp, vertical = 11.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, contentDescription = null, tint = ClipxError, modifier = Modifier.size(20.dp))
        Spacer(Modifier.width(14.dp))
        Text(label, color = ClipxError, style = MaterialTheme.typography.bodyLarge)
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HistoryActionsSheet(
    item: ClipxHistoryItem,
    onDismiss: () -> Unit,
    onCopy: () -> Unit,
    onRemove: () -> Unit,
) {
    val palette = LocalClipxPalette.current
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = false),
        containerColor = palette.surface,
        contentColor = MaterialTheme.colorScheme.onSurface,
    ) {
        Column(modifier = Modifier.fillMaxWidth().padding(horizontal = 18.dp).padding(bottom = 22.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                HistoryTypeBadge(item.type)
                Spacer(Modifier.width(12.dp))
                Column(
                    Modifier
                        .weight(1f)
                        .heightIn(max = 92.dp)
                ) {
                    Text(
                        item.preview,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurface,
                        maxLines = 3,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text("${item.sourceDevice}  •  ${item.ageLabel}", color = palette.mutedText, style = MaterialTheme.typography.bodyMedium, maxLines = 1)
                }
            }
            Spacer(Modifier.height(12.dp))
            Divider(color = palette.divider)
            HistoryActionRow("Copy to clipboard", Icons.Filled.ContentCopy, onCopy)
            HistoryActionRow("Remove from history", Icons.Filled.DeleteOutline, onRemove, danger = true)
        }
    }
}

@Composable
private fun HistoryActionRow(
    label: String,
    icon: ImageVector,
    onClick: () -> Unit,
    danger: Boolean = false,
) {
    val tint = if (danger) ClipxError else MaterialTheme.colorScheme.onSurface
    Row(
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 2.dp, vertical = 13.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(21.dp))
        Spacer(Modifier.width(13.dp))
        Text(label, color = tint, style = MaterialTheme.typography.bodyLarge)
    }
}

@Composable
fun HistoryTypeBadge(type: ClipType) {
    val palette = LocalClipxPalette.current
    val icon = when (type) {
        ClipType.TEXT -> Icons.Filled.TextFields
        ClipType.IMAGE -> Icons.Filled.Image
        ClipType.FILE -> Icons.Filled.AttachFile
    }
    val tint = when (type) {
        ClipType.TEXT -> palette.textBadge
        ClipType.IMAGE -> MaterialTheme.colorScheme.primary
        ClipType.FILE -> palette.codeBadge
    }
    Box(
        modifier = Modifier.size(40.dp).clip(RoundedCornerShape(10.dp)).background(palette.historyBadge),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = type.name.lowercase(), tint = tint, modifier = Modifier.size(20.dp))
    }
}

/**
 * UI-only presentation state. It deliberately does not alter the shared domain model.
 */
data class AvailableDeviceUi(
    val model: com.godiegh.clipx.AvailableDevice,
    val requesting: Boolean = false,
)

data class ClipxHistoryItem(
    val id: String,
    val preview: String,
    val content: String,
    val sourceDevice: String,
    val ageLabel: String,
    val type: ClipType,
)

data class ClipxSheetAction(
    val id: String,
    val label: String,
    val destructive: Boolean = false,
)

data class ClipxSheetRequest(
    val title: String,
    val message: String? = null,
    val actions: List<ClipxSheetAction>,
)

enum class ClipxSheetResultType { ACTION, DISMISSED }

data class ClipxSheetResult(
    val type: ClipxSheetResultType,
    val actionId: String? = null,
)

/**
 * Generic event-driven bottom-sheet controller. A caller supplies a callback and receives
 * ACTION or DISMISSED. Dismissing by swipe, outside tap, or Back returns DISMISSED.
 */
@Composable
fun rememberClipxSheetController(): ClipxSheetController = androidx.compose.runtime.remember { ClipxSheetController() }

class ClipxSheetController: ViewModel() {

    private val queue = ArrayDeque<PendingClipxSheet>()
    var pending: PendingClipxSheet? by mutableStateOf(null)
        private set

    /** Event-driven API for UI callers. The callback receives ACTION or DISMISSED. */
    fun show(request: ClipxSheetRequest, onResult: (ClipxSheetResult) -> Unit) {
        val item = PendingClipxSheet(request, onResult)
        if (pending == null) pending = item else queue.addLast(item)
    }

    fun dismiss() = resolve(ClipxSheetResult(ClipxSheetResultType.DISMISSED))

    fun choose(actionId: String) = resolve(ClipxSheetResult(ClipxSheetResultType.ACTION, actionId))

    private fun resolve(result: ClipxSheetResult) {
        val current = pending ?: return
        pending = null
        current.onResult(result)
        pending = queue.removeFirstOrNull()
    }
}

data class PendingClipxSheet(
    val request: ClipxSheetRequest,
    val onResult: (ClipxSheetResult) -> Unit,
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ClipxDecisionSheetHost(controller: ClipxSheetController) {
    val pending = controller.pending ?: return
    val palette = LocalClipxPalette.current
    ModalBottomSheet(
        onDismissRequest = controller::dismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = false),
        containerColor = palette.surface,
        contentColor = MaterialTheme.colorScheme.onSurface,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 20.dp, vertical = 6.dp)
                .navigationBarsPadding(),
            verticalArrangement = Arrangement.SpaceBetween,
        ) {
            Column(modifier = Modifier.fillMaxWidth()) {
                Text(pending.request.title, style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
                pending.request.message?.let {
                    Spacer(Modifier.height(10.dp))
                    Text(it, style = MaterialTheme.typography.bodyLarge, color = palette.mutedText)
                }
            }
            Spacer(Modifier.height(20.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                pending.request.actions.forEach { action ->
                    Button(
                        onClick = { controller.choose(action.id) },
                        modifier = Modifier.weight(1f),
                        shape = RoundedCornerShape(12.dp),
                        colors = if (action.destructive) {
                            ButtonDefaults.buttonColors(
                                containerColor = MaterialTheme.colorScheme.errorContainer,
                                contentColor = MaterialTheme.colorScheme.onErrorContainer,
                            )
                        } else {
                            ButtonDefaults.buttonColors(
                                containerColor = MaterialTheme.colorScheme.surfaceVariant,
                                contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        },
                    ) {
                        Text(action.label, style = MaterialTheme.typography.labelLarge, maxLines = 1)
                    }
                }
            }
        }
    }
}

private fun connectionLabel(status: ConnectionStatus): String = when (status) {
    ConnectionStatus.CONNECTED -> "Connected"
    ConnectionStatus.CONNECTING -> "Connecting…"
    ConnectionStatus.DISCONNECTED -> "Disconnected"
    ConnectionStatus.UNAVAILABLE -> "Unavailable"
}
