package com.godiegh.clipx.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.HelpOutline
import androidx.compose.material.icons.automirrored.filled.InsertDriveFile
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.DeleteForever
import androidx.compose.material.icons.filled.FileDownload
import androidx.compose.material.icons.filled.FileOpen
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.PhoneIphone
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Sensors
import androidx.compose.material.icons.filled.Sync
import androidx.compose.material.icons.filled.WifiOff
import androidx.compose.material3.DividerDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import com.godiegh.clipx.ClipType
import com.godiegh.clipx.ConnectionStatus
import com.godiegh.clipx.DeviceType
import com.godiegh.clipx.PairedDevice
import com.godiegh.clipx.SystemClipboardItem
import com.godiegh.clipx.OutgoingFile
import com.godiegh.clipx.ui.theme.LocalClipxPalette
import kotlin.time.Duration.Companion.milliseconds
import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight

@Composable
fun DevicesScreen(
    pairedDevices: List<PairedDevice>,
    availableDevices: List<AvailableDeviceUi>,
    onOpenPair: () -> Unit,
    onOpenThisDevice: () -> Unit,
    onOpenDiagnostics: () -> Unit,
    onOpenSync: () -> Unit,
    onOpenDetails: (String) -> Unit,
    onPair: (String) -> Unit,
    onConnect: (String) -> Unit,
) {
    val listState = rememberLazyListState()
    val elevated by remember { derivedStateOf { listState.canScrollBackward } }

    Column(modifier = Modifier.fillMaxSize()) {
        AppTopBar(
            title = "",
            showLogo = true,
            elevated = elevated,
            rightContent = {
                IconButton(onClick = onOpenSync) {
                    Icon(
                        Icons.AutoMirrored.Filled.Send,
                        contentDescription = "Send",
                        tint = MaterialTheme.colorScheme.onSurface,
                        modifier = Modifier.size(21.dp),
                    )
                }
                IconButton(onClick = onOpenDiagnostics) {
                    Icon(Icons.AutoMirrored.Filled.HelpOutline, contentDescription = "Help and diagnostics", tint = MaterialTheme.colorScheme.onSurface, modifier = Modifier.size(21.dp))
                }
                IconButton(onClick = onOpenThisDevice) {
                    Icon(Icons.Filled.PhoneIphone, contentDescription = "Device identity", tint = MaterialTheme.colorScheme.onSurface, modifier = Modifier.size(21.dp))
                }
            },
        )
        LazyColumn(
            state = listState,
            modifier = Modifier.weight(1f),
            contentPadding = PaddingValues(top = 2.dp, bottom = 10.dp),
        ) {
            item { SectionLabel("PAIRED DEVICES") }
            if (pairedDevices.isEmpty()) {
                item {
                    Text(
                        "No paired devices yet.",
                        color = LocalClipxPalette.current.mutedText,
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.padding(horizontal = 18.dp, vertical = 14.dp),
                    )
                }
            } else {
                items(pairedDevices, key = { "paired:${it.id}" }) { device ->
                    DeviceRow(
                        device = device,
                        onClick = { onOpenDetails(device.id) },
                        actionLabel = if (device.status == ConnectionStatus.DISCONNECTED) "Connect" else null,
                        onAction = { onConnect(device.id) },
                    )
                }
            }
            item {
                SectionLabel(
                    text = "AVAILABLE DEVICES",
                    action = {
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            modifier = Modifier.padding(vertical = 1.dp),
                        ) {
                            TextButton(onClick = onOpenPair) {
                                Icon(Icons.Filled.Sensors, contentDescription = null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(17.dp))
                                Spacer(Modifier.width(4.dp))
                                Text("Scan", color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.labelLarge)
                            }
                        }
                    },
                )
            }
            items(availableDevices, key = { "available:${it.model.id}" }) { device ->
                AvailableDeviceRow(
                    device = device,
                    onPair = { onPair(device.model.id) },
                )
            }
        }
    }
}

@Composable
fun DeviceDetailsScreen(
    device: PairedDevice?,
    onBack: () -> Unit,
    onAutoConnectChange: (Boolean) -> Unit,
    onDisconnect: () -> Unit,
    onForget: () -> Unit,
) {
    val palette = LocalClipxPalette.current
    if (device == null) {
        Column(modifier = Modifier.fillMaxSize()) {
            ScreenHeader("Device Details", onBack)
            Text("Device no longer exists.", color = palette.mutedText, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(18.dp))
        }
        return
    }

    val listState = rememberLazyListState()
    val elevated by remember { derivedStateOf { listState.canScrollBackward } }

    Column(modifier = Modifier.fillMaxSize()) {
        ScreenHeader("Device Details", onBack, elevated = elevated)
        LazyColumn(state = listState, modifier = Modifier.weight(1f), contentPadding = PaddingValues(bottom = 20.dp)) {
        item {
            Row(
                modifier = Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 7.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                DeviceAvatar(device.type, size = 64.dp)
                Spacer(Modifier.width(13.dp))
                Column(Modifier.weight(1f)) {
                    Text(
                        device.name,
                        style = MaterialTheme.typography.titleLarge,
                        color = MaterialTheme.colorScheme.onSurface,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        StatusDot(device.status)
                        Spacer(Modifier.width(7.dp))
                        Text(connectionLabel(device.status), color = palette.mutedText, style = MaterialTheme.typography.bodyMedium)
                    }
                }
            }
        }
        item {
            GlassCard(modifier = Modifier.padding(horizontal = 18.dp, vertical = 4.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(14.dp), modifier = Modifier.fillMaxWidth()) {
                    InfoPair("IP Address", device.ipAddress, Modifier.weight(1f))
                    InfoPair("Port", device.wsPort.toString(), Modifier.weight(1f))
                }
                HorizontalDivider(
                    modifier = Modifier.padding(vertical = 10.dp),
                    thickness = DividerDefaults.Thickness,
                    color = palette.divider
                )
                InfoPair("Fingerprint", formatFingerprint(device.id), Modifier.fillMaxWidth())
            }
        }
        item {
            GlassCard(modifier = Modifier.padding(horizontal = 18.dp, vertical = 4.dp)) {
                ToggleRow(
                    label = "Auto-connect",
                    description = "Connect automatically when device is available",
                    checked = device.autoConnect,
                    onCheckedChange = onAutoConnectChange,
                )
            }
        }
        item {
            GlassCard(modifier = Modifier.padding(horizontal = 18.dp, vertical = 4.dp)) {
                if (device.status == ConnectionStatus.CONNECTED) {
                    DangerRow("Disconnect", Icons.Filled.WifiOff, onDisconnect)
                    HorizontalDivider(Modifier, DividerDefaults.Thickness, color = palette.divider)
                }
                DangerRow("Forget Device", Icons.Filled.DeleteForever, onForget)
            }
        }
        }
    }
}

@Composable
fun PairNewDeviceScreen(
    availableDevices: List<AvailableDeviceUi>,
    onBack: () -> Unit,
    requestId: String?,
    onPair: (String) -> Unit,
    onCantSeeDevice: () -> Unit,
) {
    val palette = LocalClipxPalette.current
    val listState = rememberLazyListState()
    val elevated by remember { derivedStateOf { listState.canScrollBackward } }

    Column(modifier = Modifier.fillMaxSize()) {
        ScreenHeader("Pair New Device", onBack, elevated = elevated)
        LazyColumn(state = listState, modifier = Modifier.weight(1f), contentPadding = PaddingValues(bottom = 18.dp)) {
        item {
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                modifier = Modifier.fillMaxWidth().padding(top = 2.dp),
            ) {
                ScanRadar()
                Text("Discovered Nearby Devices", style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onBackground, modifier = Modifier.padding(top = 1.dp))
                Text(
                    "Make sure the other device has Clipx app open and is reachable.",
                    color = palette.mutedText,
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.padding(horizontal = 48.dp, vertical = 5.dp),
                )
            }
        }
        item { Spacer(Modifier.height(14.dp)) }
        item { SectionLabel("FOUND DEVICES") }
        items(availableDevices, key = { it.model.id }) { device ->
            AvailableDeviceRow(
                device = device.copy(requesting = requestId == device.model.id),
                onPair = { onPair(device.model.id) },
            )
        }
        item {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.Center,
            ) {
                TextButton(onClick = onCantSeeDevice) {
                    Text(
                        "Can’t see your device?",
                        color = MaterialTheme.colorScheme.primary,
                        style = MaterialTheme.typography.labelLarge,
                    )
                }
            }
        }
        }
    }
}

@Composable
fun ThisDeviceScreen(identity: com.godiegh.clipx.ThisDeviceInfo?, onBack: () -> Unit) {
    val palette = LocalClipxPalette.current
    val listState = rememberLazyListState()
    val elevated by remember { derivedStateOf { listState.canScrollBackward } }

    Column(modifier = Modifier.fillMaxSize()) {
        ScreenHeader("Device Identity", onBack, elevated = elevated)
        LazyColumn(state = listState, modifier = Modifier.weight(1f), contentPadding = PaddingValues(bottom = 20.dp)) {
        item {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.padding(horizontal = 18.dp, vertical = 7.dp),
            ) {
                DeviceAvatar(identity?.deviceType ?: DeviceType.ANDROID, size = 66.dp)
                Spacer(Modifier.width(14.dp))
                Column(Modifier.weight(1f)) {
                    Text(
                        identity?.name ?: "This device",
                        style = MaterialTheme.typography.titleLarge,
                        color = MaterialTheme.colorScheme.onSurface,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        "${(identity?.deviceType ?: DeviceType.ANDROID).name.lowercase().replaceFirstChar { it.uppercase() }} • ${identity?.name ?: "Unknown"}",
                        color = palette.mutedText,
                        style = MaterialTheme.typography.bodyMedium,
                        maxLines = 1,
                    )
                }
            }
        }
        item {
            GlassCard(modifier = Modifier.padding(horizontal = 18.dp, vertical = 4.dp)) {
                InfoPair("Device Name", identity?.name ?: "Unknown", Modifier.fillMaxWidth())
                HorizontalDivider(
                    modifier = Modifier.padding(vertical = 10.dp),
                    thickness = DividerDefaults.Thickness,
                    color = palette.divider
                )
                Row(horizontalArrangement = Arrangement.spacedBy(18.dp), modifier = Modifier.fillMaxWidth()) {
                    InfoPair("IP Address", identity?.ipAddress ?: "Unknown", Modifier.weight(1f))
                    InfoPair("Port", identity?.wsPort?.toString() ?: "Unknown", Modifier.weight(1f))
                }
                HorizontalDivider(
                    modifier = Modifier.padding(vertical = 10.dp),
                    thickness = DividerDefaults.Thickness,
                    color = palette.divider
                )
                InfoPair("Fingerprint", formatFingerprint(identity?.fingerprint.orEmpty()))
            }
        }
        }
    }
}

private data class DiscoveryGuideStep(
    val number: Int,
    val title: String,
    val summary: String,
    val details: String,
)

private val discoveryGuideSteps = listOf(
    DiscoveryGuideStep(
        number = 1,
        title = "Connect both devices",
        summary = "Use the same Wi-Fi network for normal discovery.",
        details = "Connect this phone and the other device to the same Wi-Fi network. Both devices need to be able to communicate over that network."
    ),
    DiscoveryGuideStep(
        number = 2,
        title = "Keep ClipX running",
        summary = "Discovery happens automatically. There is no Scan button.",
        details = "ClipX automatically announces its presence and listens for other ClipX devices. Keep ClipX running on both devices while you wait for them to discover each other."
    ),
    DiscoveryGuideStep(
        number = 3,
        title = "Devices discover each other",
        summary = "Reachable devices are detected automatically.",
        details = "When this phone receives an announcement from another ClipX device, the core identifies it and records it as an available device."
    ),
    DiscoveryGuideStep(
        number = 4,
        title = "The list updates automatically",
        summary = "The discovered device appears under Available Devices.",
        details = "When the available-device list changes, ClipX notifies the app. The app fetches the latest list and displays the device. You don't need to scan or manually refresh."
    ),
    DiscoveryGuideStep(
        number = 5,
        title = "Pair your device",
        summary = "Tap Pair beside the device you want to connect.",
        details = "Follow the pairing prompts to establish trust between the two devices. Once pairing succeeds, the device moves into your Paired Devices list."
    ),
    DiscoveryGuideStep(
        number = 6,
        title = "Availability stays up to date",
        summary = "Devices can disappear and return automatically.",
        details = "If a device stops announcing its presence for long enough, ClipX treats it as unavailable and removes it from the nearby list. When it becomes reachable and announces itself again, it will appear in the list."
    ),
)

@Composable
fun CantSeeDeviceScreen(onBack: () -> Unit) {
    val palette = LocalClipxPalette.current
    val listState = rememberLazyListState()
    val elevated by remember {
        derivedStateOf { listState.canScrollBackward }
    }

    Column(modifier = Modifier.fillMaxSize()) {
        ScreenHeader(
            "Can’t see your device?",
            onBack,
            elevated = elevated,
        )

        LazyColumn(
            state = listState,
            modifier = Modifier.weight(1f),
            contentPadding = PaddingValues(
                horizontal = 18.dp,
                vertical = 16.dp,
            ),
        ) {
            // Introduction
            item {
                GlassCard {
                    Column {
                        Text(
                            text = "Devices appear automatically",
                            style = MaterialTheme.typography.titleLarge,
                            fontWeight = FontWeight.SemiBold,
                            color = MaterialTheme.colorScheme.onSurface,
                        )

                        Spacer(Modifier.height(8.dp))

                        Text(
                            text = "ClipX continuously discovers other running ClipX devices on your local network. You don't need to start a scan.",
                            style = MaterialTheme.typography.bodyMedium,
                            color = palette.mutedText,
                        )

                        Spacer(Modifier.height(12.dp))

                        Text(
                            text = "Make sure both devices are connected to the same Wi-Fi network and keep ClipX running. Follow the steps below to understand what happens next.",
                            style = MaterialTheme.typography.bodySmall,
                            color = palette.mutedText,
                        )
                    }
                }

                Spacer(Modifier.height(24.dp))
            }

            // Automatic discovery lifecycle
            item {
                Text(
                    text = "How automatic discovery works",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.onSurface,
                )

                Spacer(Modifier.height(6.dp))

                Text(
                    text = "Expand any step to see what happens.",
                    style = MaterialTheme.typography.bodySmall,
                    color = palette.mutedText,
                )

                Spacer(Modifier.height(16.dp))
            }

            itemsIndexed(
                items = discoveryGuideSteps,
                key = { _, step -> step.number },
            ) { index, step ->
                DiscoveryTimelineStep(
                    step = step,
                    isLast = index == discoveryGuideSteps.lastIndex,
                    initiallyExpanded = index == 0,
                    mutedText = palette.mutedText,
                )
            }

            // Experimental Wi-Fi Direct information
            item {
                Spacer(Modifier.height(12.dp))

                ExperimentalWifiDirectCard(
                    mutedText = palette.mutedText,
                )

                Spacer(Modifier.height(16.dp))
            }
        }
    }
}

@Composable
private fun DiscoveryTimelineStep(
    step: DiscoveryGuideStep,
    isLast: Boolean,
    initiallyExpanded: Boolean,
    mutedText: Color,
) {
    var expanded by rememberSaveable(step.number) {
        mutableStateOf(initiallyExpanded)
    }

    val lineColor = MaterialTheme.colorScheme.outlineVariant
    val numberBackground = MaterialTheme.colorScheme.primaryContainer
    val numberColor = MaterialTheme.colorScheme.onPrimaryContainer

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(IntrinsicSize.Min),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.Top,
    ) {

        // Number and connecting line
        Column(
            modifier = Modifier
                .width(32.dp)
                .fillMaxHeight(),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Box(
                modifier = Modifier
                    .size(30.dp)
                    .clip(CircleShape)
                    .background(numberBackground),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = step.number.toString(),
                    style = MaterialTheme.typography.labelLarge,
                    fontWeight = FontWeight.Bold,
                    color = numberColor,
                )
            }

            if (!isLast) {
                Spacer(
                    modifier = Modifier
                        .width(2.dp)
                        .weight(1f)
                        .background(lineColor),
                )
            }
        }

        // Expandable content
        Column(
            modifier = Modifier.weight(1f),
        ) {
            Card(
                modifier = Modifier
                    .fillMaxWidth()
                    .animateContentSize()
                    .clickable(
                        role = Role.Button,
                        onClickLabel = if (expanded) {
                            "Collapse ${step.title}"
                        } else {
                            "Expand ${step.title}"
                        },
                    ) {
                        expanded = !expanded
                    },
                shape = RoundedCornerShape(18.dp),
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.surfaceVariant
                        .copy(alpha = 0.45f),
                ),
                border = BorderStroke(
                    1.dp,
                    MaterialTheme.colorScheme.outlineVariant.copy(
                        alpha = 0.45f,
                    ),
                ),
                elevation = CardDefaults.cardElevation(
                    defaultElevation = 0.dp,
                ),
            ) {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(16.dp),
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Column(
                            modifier = Modifier.weight(1f),
                        ) {
                            Text(
                                text = step.title,
                                style = MaterialTheme.typography.titleSmall,
                                fontWeight = FontWeight.SemiBold,
                                color = MaterialTheme.colorScheme.onSurface,
                            )

                            Spacer(Modifier.height(4.dp))

                            Text(
                                text = step.summary,
                                style = MaterialTheme.typography.bodySmall,
                                color = mutedText,
                            )
                        }

                        Spacer(Modifier.width(8.dp))

                        ExpandChevron(expanded = expanded)
                    }

                    if (expanded) {
                        Spacer(Modifier.height(14.dp))

                        HorizontalDivider(
                            color = MaterialTheme.colorScheme.outlineVariant
                                .copy(alpha = 0.5f),
                        )

                        Spacer(Modifier.height(12.dp))

                        Text(
                            text = step.details,
                            style = MaterialTheme.typography.bodyMedium,
                            color = mutedText,
                        )
                    }
                }
            }

            // This space is also occupied by the connecting line.
            if (!isLast) {
                Spacer(Modifier.height(12.dp))
            }
        }
    }
}

@Composable
private fun ExpandChevron(expanded: Boolean) {
    val color = MaterialTheme.colorScheme.onSurfaceVariant

    Canvas(
        modifier = Modifier.size(22.dp),
    ) {
        val path = Path().apply {
            if (expanded) {
                moveTo(size.width * 0.25f, size.height * 0.60f)
                lineTo(size.width * 0.50f, size.height * 0.40f)
                lineTo(size.width * 0.75f, size.height * 0.60f)
            } else {
                moveTo(size.width * 0.25f, size.height * 0.40f)
                lineTo(size.width * 0.50f, size.height * 0.60f)
                lineTo(size.width * 0.75f, size.height * 0.40f)
            }
        }

        drawPath(
            path = path,
            color = color,
            style = Stroke(
                width = 2.dp.toPx(),
                cap = StrokeCap.Round,
                join = StrokeJoin.Round,
            ),
        )
    }
}

@Composable
private fun ExperimentalWifiDirectCard(
    mutedText: Color,
) {
    var expanded by rememberSaveable {
        mutableStateOf(false)
    }

    GlassCard {
        Column(
            modifier = Modifier.fillMaxWidth(),
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable(
                        role = Role.Button,
                        onClickLabel = if (expanded) {
                            "Collapse Wi-Fi Direct information"
                        } else {
                            "Expand Wi-Fi Direct information"
                        },
                    ) {
                        expanded = !expanded
                    },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(
                    modifier = Modifier.weight(1f),
                ) {
                    Text(
                        text = "Wi-Fi Direct",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                        color = MaterialTheme.colorScheme.onSurface,
                    )

                    Spacer(Modifier.height(4.dp))

                    Text(
                        text = "An experimental connection option",
                        style = MaterialTheme.typography.bodySmall,
                        color = mutedText,
                    )
                }

                ExpandChevron(expanded = expanded)
            }

            if (expanded) {
                Spacer(Modifier.height(12.dp))

                Text(
                    text = "Some ClipX builds also support an experimental Wi-Fi Direct link. You can try it as an alternative when normal local-network connectivity is unavailable. However, support and reliability can vary between devices.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = mutedText,
                )
            }
        }
    }
}

@Composable
fun SendScreen(
    item: SystemClipboardItem?,
    pendingFiles: List<OutgoingFile>,
    connectedDeviceCount: Int,
    autoSyncOnResume: Boolean,
    onAutoSyncChange: (Boolean) -> Unit,
    onRefresh: () -> Unit,
    onPickFiles: () -> Unit,
    onRemovePendingFile: (Int) -> Unit,
    onSend: (SystemClipboardItem?) -> Unit,
    onSendFiles: () -> Unit,
    sendingFiles: Boolean,
    onBack: () -> Unit,
    onClearClipboard: () -> Unit,
) {
    val listState = rememberLazyListState()
    val elevated by remember { derivedStateOf { listState.canScrollBackward } }
    val clipboardReady = item?.canSend == true
    val readyCount = (if (clipboardReady) 1 else 0) + pendingFiles.size
    val canSend = connectedDeviceCount > 0 && readyCount > 0
    val palette = LocalClipxPalette.current

    Column(modifier = Modifier.fillMaxSize()) {
        AppTopBar(
            title = "Send",
            onBack = onBack,
            elevated = elevated,
        )

        LazyColumn(
            state = listState,
            modifier = Modifier.weight(1f),
            contentPadding = PaddingValues(bottom = 20.dp),
        ) {
            item {
                SectionLabel("AUTO-SYNC")
                GlassCard(modifier = Modifier.padding(horizontal = 18.dp, vertical = 4.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Icon(Icons.Filled.Sync, contentDescription = null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(20.dp))
                        Spacer(Modifier.width(12.dp))
                        Text("Sync clipboard when Clipx opens", modifier = Modifier.weight(1f), style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurface)
                        Spacer(Modifier.width(10.dp))
                        Switch(
                            checked = autoSyncOnResume,
                            onCheckedChange = onAutoSyncChange,
                            colors = SwitchDefaults.colors(uncheckedThumbColor = Color(0xFF5F6875)),
                        )
                    }
                }
                /**
                 * TODO: Later we'll add a way to auto-sync when activity is not visible. aware of what we know about android limitations.
                 * I think of making the activity or another an overlay on top of other apps but at user consent.
                 * User explicitly enables auto sync with this toggle, which already does it,
                 * to mean auto send clipboards as the activity is visible in some ways.
                 * And in the tile; just clicking below the tile just at the name(tile title) opens a more options screen(if we can make android give it,
                 * like it does to some tiles. eg. WI-FI one. it is a builtin but I think android can make us write our own too if that is possible or if not we can write a screen/activity
                 * that clicking the tile opens it).
                 * We can add an add an overlay over other app toggle to toggle the overlay app button that tricks clipx like it in
                 * background and auto-sending but the truth is user just focused it to make it enough visible this above feature
                 * to work on it is own
                 * */
            }

            item {
                GlassCard(modifier = Modifier.padding(horizontal = 18.dp, vertical = 4.dp)) {
                    Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Text("Clipboard", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurface, modifier = Modifier.weight(1f))
                        IconButton(onClick = onRefresh) { Icon(Icons.Filled.Refresh, contentDescription = "Refresh clipboard", tint = MaterialTheme.colorScheme.onSurfaceVariant) }
                    }
                    if (item == null) {
                        Row(modifier = Modifier.fillMaxWidth().clickable(onClick = onRefresh).padding(vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                            Icon(Icons.Filled.ContentCopy, contentDescription = null, tint = palette.mutedText, modifier = Modifier.size(20.dp))
                            Spacer(Modifier.width(10.dp))
                            Text("Add clipboard", style = MaterialTheme.typography.bodyMedium, color = palette.mutedText)
                        }
                    } else {
                        CurrentClipboardItem(
                            item = item,
                            onClear = onClearClipboard,
                        )
                    }
                }
            }

            item {
                SectionLabel(
                    "FILES",
                    action = {
                        TextButton(onClick = onPickFiles) {
                            Icon(Icons.Filled.FileOpen, contentDescription = null, modifier = Modifier.size(17.dp))
                            Spacer(Modifier.width(4.dp))
                            Text("Add files", style = MaterialTheme.typography.labelMedium)
                        }
                    },
                )
            }
            if (pendingFiles.isEmpty()) {
                item {
                    GlassCard(modifier = Modifier.padding(horizontal = 18.dp, vertical = 4.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Icon(Icons.AutoMirrored.Filled.InsertDriveFile, contentDescription = null, tint = palette.mutedText, modifier = Modifier.size(20.dp))
                            Spacer(Modifier.width(10.dp))
                            Text("No files selected", style = MaterialTheme.typography.bodyMedium, color = palette.mutedText)
                        }
                    }
                }
            } else {
                items(pendingFiles.indices.toList(), key = { it }) { index ->
                    val file = pendingFiles[index]
                    GlassCard(modifier = Modifier.padding(horizontal = 18.dp, vertical = 4.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                            Icon(Icons.AutoMirrored.Filled.InsertDriveFile, contentDescription = null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(21.dp))
                            Spacer(Modifier.width(11.dp))
                            Column(Modifier.weight(1f)) {
                                Text(file.name, maxLines = 1, overflow = TextOverflow.Ellipsis, color = MaterialTheme.colorScheme.onSurface, style = MaterialTheme.typography.bodyMedium)
                                Text("${file.mimeType}  ·  ${formatFileSize(file.size)}", color = palette.mutedText, style = MaterialTheme.typography.labelSmall)
                            }
                            IconButton(onClick = { onRemovePendingFile(index) }) {
                                Icon(Icons.Filled.Clear, contentDescription = "Remove ${file.name}", tint = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        }
                    }
                }
            }

            item {
                Spacer(Modifier.height(6.dp))
            }
        }

        Surface(
            modifier = Modifier.fillMaxWidth(),
            color = palette.navigationSurface,
            tonalElevation = 2.dp,
            border = androidx.compose.foundation.BorderStroke(1.dp, palette.cardBorder),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(
                        when {
                            connectedDeviceCount == 0 -> "Connect a device to send"
                            readyCount == 0 -> "Nothing selected"
                            else -> "$readyCount ready to send"
                        },
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                    Text(
                        when {
                            pendingFiles.isNotEmpty() && clipboardReady -> "${pendingFiles.size} file${if (pendingFiles.size == 1) "" else "s"} + clipboard"
                            pendingFiles.isNotEmpty() -> "${pendingFiles.size} selected file${if (pendingFiles.size == 1) "" else "s"}"
                            clipboardReady -> "Current clipboard selected"
                            else -> "Choose content above"
                        },
                        style = MaterialTheme.typography.labelSmall,
                        color = palette.mutedText,
                    )
                }
                Spacer(Modifier.width(12.dp))
                Button(
                    onClick = {
                        if (clipboardReady) onSend(item)
                        if (pendingFiles.isNotEmpty()) onSendFiles()
                    },
                    enabled = canSend && !sendingFiles,
                    shape = RoundedCornerShape(12.dp),
                ) {
                    if (sendingFiles) { androidx.compose.material3.CircularProgressIndicator(modifier = Modifier.size(17.dp), strokeWidth = 2.dp) } else { Icon(
                        Icons.AutoMirrored.Filled.Send, contentDescription = null, modifier = Modifier.size(18.dp)) }
                    Spacer(Modifier.width(7.dp))
                    Text(if (sendingFiles) "Preparing…" else "Send")
                }
            }
        }
    }
}

@Composable
fun DiagnosticsScreen(
    logs: List<String>,
    onBack: () -> Unit,
    onRefresh: () -> Unit,
    onClear: () -> Unit,
) {
    val palette = LocalClipxPalette.current
    val clipboardManager = LocalClipboardManager.current
    val listState = rememberLazyListState()
    val elevated by remember { derivedStateOf { listState.canScrollBackward } }
    LaunchedEffect(Unit) { while (true) { onRefresh(); delay(1_000.milliseconds) } }
    LaunchedEffect(logs.lastOrNull()) { if (logs.isNotEmpty()) listState.animateScrollToItem(logs.lastIndex) }
    Column(modifier = Modifier.fillMaxSize()) {
        ScreenHeader(
            "Help & diagnostics",
            onBack,
            elevated = elevated,
            rightContent = {
                IconButton(onClick = onRefresh) {
                    Icon(Icons.Filled.Refresh, contentDescription = "Refresh diagnostics", tint = MaterialTheme.colorScheme.onSurface)
                }
                IconButton(onClick = { clipboardManager.setText(AnnotatedString(logs.joinToString("\n"))) }, enabled = logs.isNotEmpty()) {
                    Icon(Icons.Filled.ContentCopy, contentDescription = "Copy diagnostic logs", tint = MaterialTheme.colorScheme.onSurface)
                }
                IconButton(onClick = onClear, enabled = logs.isNotEmpty()) {
                    Icon(Icons.Filled.DeleteForever, contentDescription = "Clear diagnostic logs", tint = MaterialTheme.colorScheme.onSurface)
                }
            },
        )
        GlassCard(modifier = Modifier.padding(horizontal = 18.dp, vertical = 5.dp)) {
            Text("Live diagnostic logs", style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurface)
            Spacer(Modifier.height(5.dp))
            Text("This in-memory feed keeps up to 250 recent ClipX core/bridge events while the app process is running. It is not the full Android system log. Review entries before sharing them.", style = MaterialTheme.typography.bodySmall, color = palette.mutedText)
        }
        LazyColumn(state = listState, modifier = Modifier.weight(1f).fillMaxWidth(), contentPadding = PaddingValues(start = 12.dp, end = 12.dp, top = 4.dp, bottom = 16.dp)) {
            if (logs.isEmpty()) item {
                Text("Waiting for ClipX events…", style = MaterialTheme.typography.bodyMedium, color = palette.mutedText, modifier = Modifier.padding(horizontal = 8.dp, vertical = 16.dp))
            } else items(logs) { line ->
                SelectionContainer {
                    Text(line, style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace), color = MaterialTheme.colorScheme.onSurface, modifier = Modifier.fillMaxWidth().padding(horizontal = 6.dp, vertical = 3.dp))
                }
            }
        }
    }
}


@Composable
private fun CurrentClipboardItem(
    item: SystemClipboardItem,
    onClear: () -> Unit,
) {
    val palette = LocalClipxPalette.current

    GlassCard(
        modifier = Modifier
            .padding(horizontal = 18.dp, vertical = 4.dp)
            .border(
                1.dp,
                MaterialTheme.colorScheme.primary,
                RoundedCornerShape(12.dp),
            )
            .clickable(onClick = onClear),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Icon(
                when {
                    item.isImage -> Icons.Filled.Image
                    item.isFile -> Icons.AutoMirrored.Filled.InsertDriveFile
                    else -> Icons.Filled.ContentCopy
                },
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
                modifier = Modifier.size(22.dp),
            )

            Spacer(Modifier.width(12.dp))

            Column(Modifier.weight(1f)) {
                Text(
                    item.fileName
                        ?: item.text
                        ?: if (item.isImage) {
                            "Image clipboard content"
                        } else {
                            "Unsupported clipboard content"
                        },
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis,
                )

                Spacer(Modifier.height(4.dp))

                Text(
                    when {
                        item.isFile -> "File • ${formatFileSize(item.fileSize)}"
                        item.isImage -> "Image • sendable"
                        item.isRichText -> "Rich text • sendable"
                        item.isText -> "Text • sendable"
                        else -> item.mimeTypes.joinToString().ifBlank { "Unsupported" }
                    },
                    style = MaterialTheme.typography.labelSmall,
                    color = palette.mutedText,
                )
            }

            if (item.canSend) {
                Icon(
                    Icons.Filled.Clear,
                    contentDescription = "Remove clipboard",
                    tint = MaterialTheme.colorScheme.primary,
                    modifier = Modifier
                        .padding(start = 10.dp)
                        .size(20.dp),
                )
            }
        }
    }
}

private fun formatFileSize(size: Long): String = when {
    size >= 1L shl 30 -> "%.1f GB".format(size.toDouble() / (1L shl 30))
    size >= 1L shl 20 -> "%.1f MB".format(size.toDouble() / (1L shl 20))
    size >= 1L shl 10 -> "%.0f KB".format(size.toDouble() / (1L shl 10))
    else -> "$size B"
}

@Composable
fun HistoryScreen(
    items: List<ClipxHistoryItem>,
    search: Boolean,
    fileTransfers: Map<String, com.godiegh.clipx.FileTransferUi>,
    onSearchChange: (Boolean) -> Unit,
    onBack: () -> Unit,
    onClearAll: () -> Unit,
    onItemActions: (ClipxHistoryItem) -> Unit,
    onCopyItem: (ClipxHistoryItem) -> Unit,
) {
    var query by remember { mutableStateOf("") }
    val palette = LocalClipxPalette.current
    val filtered = items.filter {
        query.isBlank() || it.content.contains(query, ignoreCase = true) || it.sourceDevice.contains(query, ignoreCase = true)
    }
    val listState = rememberLazyListState()
    val elevated by remember { derivedStateOf { listState.canScrollBackward } }

    Column(modifier = Modifier.fillMaxSize()) {
        if (search) {
            TopBarSurface(elevated = elevated) {
                IconButton(onClick = { query = ""; onSearchChange(false) }) {
                    Icon(Icons.Filled.Clear, contentDescription = "Close search", tint = MaterialTheme.colorScheme.onSurface)
                }
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    modifier = Modifier.weight(1f),
                    singleLine = true,
                    shape = RoundedCornerShape(50),
                    placeholder = { Text("Search history", color = palette.mutedText, style = MaterialTheme.typography.bodyMedium) },
                    trailingIcon = if (query.isNotEmpty()) {
                        { IconButton(onClick = { query = "" }) { Icon(Icons.Filled.Clear, contentDescription = "Clear search", tint = palette.mutedText) } }
                    } else null,
                    colors = OutlinedTextFieldDefaults.colors(
                        focusedTextColor = MaterialTheme.colorScheme.onBackground,
                        unfocusedTextColor = MaterialTheme.colorScheme.onBackground,
                        cursorColor = MaterialTheme.colorScheme.primary,
                        focusedBorderColor = MaterialTheme.colorScheme.primary,
                        unfocusedBorderColor = palette.cardBorder,
                        focusedContainerColor = palette.surface,
                        unfocusedContainerColor = palette.surface,
                    ),
                    textStyle = MaterialTheme.typography.bodyMedium,
                )
            }
        } else {
            AppTopBar(
                title = "Clipboard History",
                onBack = null,
                elevated = elevated,
                rightContent = {
                    IconButton(onClick = { onSearchChange(true) }) {
                        Icon(Icons.Filled.Search, contentDescription = "Search history", tint = MaterialTheme.colorScheme.onSurface)
                    }
                    TextButton(onClick = onClearAll, enabled = items.isNotEmpty()) {
                        Text("Clear all", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
                    }
                },
            )
        }
        LazyColumn(state = listState, modifier = Modifier.weight(1f), contentPadding = PaddingValues(bottom = 18.dp)) {
            item {
                Text("${filtered.size} items", color = palette.mutedText, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(horizontal = 18.dp, vertical = 2.dp))
            }
            items(filtered, key = { it.id }) { item ->
                HistoryListItem(item, transfer = fileTransfers[item.id], onCopy = { onCopyItem(item) }, onActions = { onItemActions(item) })
            }
        }
    }
}

@Composable
private fun HistoryListItem(
    item: ClipxHistoryItem,
    transfer: com.godiegh.clipx.FileTransferUi?,
    onCopy: () -> Unit,
    onActions: () -> Unit,
) {
    val palette = LocalClipxPalette.current
    val transferActive = transfer?.state == "requesting" || transfer?.state == "receiving" || transfer?.state == "sending" || transfer?.state == "saving"
    val transferPercent = transfer?.let { if (it.total > 0) ((it.done.toFloat() / it.total.toFloat()).coerceIn(0f, 1f)) else 0f } ?: 0f
    val downloadDisabled = item.fileDownloaded || transferActive
    var offerExpired by remember(item.id, item.fileExpiresAtMs, item.fileDownloaded) {
        mutableStateOf(item.type == ClipType.FILE && !item.fileDownloaded && item.fileExpiresAtMs > 0L && System.currentTimeMillis() >= item.fileExpiresAtMs)
    }
    LaunchedEffect(item.id, item.fileExpiresAtMs, item.fileDownloaded) {
        if (item.type == ClipType.FILE && !item.fileDownloaded && item.fileExpiresAtMs > 0L) {
            val remaining = item.fileExpiresAtMs - System.currentTimeMillis()
            if (remaining > 0L) delay(remaining.milliseconds)
            offerExpired = true
        }
    }
    GlassCard(modifier = Modifier.padding(horizontal = 18.dp, vertical = 4.dp)) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.fillMaxWidth().height(76.dp),
        ) {
            HistoryTypeBadge(item.type)
            Spacer(Modifier.width(10.dp))
            Column(Modifier.weight(1f).height(54.dp), verticalArrangement = Arrangement.Center) {
                Text(
                    item.preview,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    "${item.sourceDevice}  •  ${item.ageLabel}",
                    color = palette.mutedText,
                    style = MaterialTheme.typography.labelSmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(top = 3.dp),
                )
            }
            val downloading = item.type == ClipType.FILE && transferActive
            IconButton(onClick = onCopy, enabled = !offerExpired && !downloadDisabled, modifier = Modifier.size(40.dp)) {
                when {
                    item.type != ClipType.FILE -> Icon(Icons.Filled.ContentCopy, contentDescription = "Copy", tint = MaterialTheme.colorScheme.onSurface)
                    downloading -> androidx.compose.material3.CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                    item.fileDownloaded -> Icon(Icons.Filled.FileOpen, contentDescription = "Downloaded", tint = MaterialTheme.colorScheme.primary)
                    else -> Icon(Icons.Filled.FileDownload, contentDescription = "Download file", tint = MaterialTheme.colorScheme.onSurface)
                }
            }
            IconButton(onClick = onActions, modifier = Modifier.size(36.dp)) {
                Icon(Icons.Filled.MoreVert, contentDescription = "Item actions", tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        if (item.type == ClipType.FILE) {
            when {
                transfer?.state == "requesting" || transfer?.state == "saving" -> {
                    androidx.compose.material3.LinearProgressIndicator(
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 4.dp, vertical = 5.dp),
                    )
                }
                else -> {
                    androidx.compose.material3.LinearProgressIndicator(
                        progress = { if (item.fileDownloaded || transfer?.state == "complete") 1f else transferPercent },
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 4.dp, vertical = 5.dp),
                    )
                }
            }
            Text(
                when {
                    transfer?.state == "requesting" -> "Starting download…"
                    transfer?.state == "receiving" -> "Downloading · ${"%.0f".format(transferPercent * 100)}%"
                    transfer?.state == "saving" -> "Saving file…"
                    transfer?.state == "expired" -> "Offer expired"
                    item.fileDownloaded -> "Download finished"
                    transfer?.state == "complete" -> "Download finished"
                    transfer?.state == "failed" -> "Download failed · tap to retry"
                    else -> "Available for download for up to 24 hours"
                },
                style = MaterialTheme.typography.labelSmall,
                color = palette.mutedText,
                modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp),
            )
        }
    }
}

private fun connectionLabel(status: ConnectionStatus): String = when (status) {
    ConnectionStatus.CONNECTED -> "Connected"
    ConnectionStatus.CONNECTING -> "Connecting…"
    ConnectionStatus.DISCONNECTED -> "Disconnected"
    ConnectionStatus.UNAVAILABLE -> "Unavailable"
}
