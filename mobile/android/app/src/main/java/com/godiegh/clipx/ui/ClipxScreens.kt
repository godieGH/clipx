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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.DeleteForever
import androidx.compose.material.icons.filled.FileDownload
import androidx.compose.material.icons.filled.Fingerprint
import androidx.compose.material.icons.filled.FontDownload
import androidx.compose.material.icons.filled.Sensors
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.WifiOff
import androidx.compose.material3.DividerDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.godiegh.clipx.ClipType
import com.godiegh.clipx.ConnectionStatus
import com.godiegh.clipx.DeviceType
import com.godiegh.clipx.PairedDevice
import com.godiegh.clipx.ui.theme.LocalClipxPalette

@Composable
fun DevicesScreen(
    pairedDevices: List<PairedDevice>,
    availableDevices: List<AvailableDeviceUi>,
    onOpenPair: () -> Unit,
    onOpenThisDevice: () -> Unit,
    onOpenDetails: (String) -> Unit,
    onPair: (String) -> Unit,
    onConnect: (String) -> Unit,
) {
    val listState = rememberLazyListState()
    val elevated by remember { derivedStateOf { listState.canScrollBackward } }

    Column(modifier = Modifier.fillMaxSize()) {
        AppTopBar(
            title = "Clipx",
            showLogo = true,
            elevated = elevated,
            rightContent = {
                IconButton(onClick = onOpenThisDevice) {
                    Icon(
                        Icons.Filled.Fingerprint,
                        contentDescription = "This device identity",
                        tint = MaterialTheme.colorScheme.onSurface,
                        modifier = Modifier.size(21.dp),
                    )
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
                items(pairedDevices, key = { it.id }) { device ->
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
            items(availableDevices, key = { it.model.id }) { device ->
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
                Text("Scanning for Clipx devices…", style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onBackground, modifier = Modifier.padding(top = 1.dp))
                Text(
                    "Make sure the other device has Clipx app open and is discoverable.",
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
        ScreenHeader("This Device (Identity)", onBack, elevated = elevated)
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

@Composable
fun CantSeeDeviceScreen(onBack: () -> Unit) {
    val palette = LocalClipxPalette.current
    val listState = rememberLazyListState()
    val elevated by remember { derivedStateOf { listState.canScrollBackward } }

    Column(modifier = Modifier.fillMaxSize()) {
        ScreenHeader("Can’t see your device?", onBack, elevated = elevated)
        LazyColumn(
            state = listState,
            modifier = Modifier.weight(1f),
            contentPadding = PaddingValues(horizontal = 18.dp, vertical = 16.dp),
        ) {
            item {
                GlassCard {
                    Text("Connect over the same Wi-Fi", style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
                    Spacer(Modifier.height(12.dp))
                    Text(
                        "For now Clipx discovers devices on your local network. Put this phone and the other device on the same Wi-Fi network, then keep Clipx open on both devices.",
                        color = palette.mutedText,
                        style = MaterialTheme.typography.bodyLarge,
                    )
                    Spacer(Modifier.height(16.dp))
                    Text("1. Join the same Wi-Fi network on both devices.", color = MaterialTheme.colorScheme.onSurface, style = MaterialTheme.typography.bodyMedium)
                    Spacer(Modifier.height(8.dp))
                    Text("2. Open Clipx on the other device and keep it running.", color = MaterialTheme.colorScheme.onSurface, style = MaterialTheme.typography.bodyMedium)
                    Spacer(Modifier.height(8.dp))
                    Text("3. Return here and open Scan again.", color = MaterialTheme.colorScheme.onSurface, style = MaterialTheme.typography.bodyMedium)
                    Spacer(Modifier.height(12.dp))
                    Text("Wi-Fi Direct support can be added later without changing this pairing flow.", color = palette.mutedText, style = MaterialTheme.typography.bodySmall)
                }
            }
        }
    }
}

@Composable
fun HistoryScreen(
    items: List<ClipxHistoryItem>,
    search: Boolean,
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
                HistoryListItem(item, onCopy = { onCopyItem(item) }, onActions = { onItemActions(item) })
            }
        }
    }
}

@Composable
private fun HistoryListItem(
    item: ClipxHistoryItem,
    onCopy: () -> Unit,
    onActions: () -> Unit,
) {
    val palette = LocalClipxPalette.current
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
            IconButton(onClick = onCopy, modifier = Modifier.size(40.dp)) {
                if (item.type == ClipType.FILE) Icon(Icons.Filled.FileDownload, contentDescription = "File Download", tint = MaterialTheme.colorScheme.onSurface) else Icon(Icons.Filled.ContentCopy, contentDescription = "Copy", tint = MaterialTheme.colorScheme.onSurface)
            }
            IconButton(onClick = onActions, modifier = Modifier.size(36.dp)) {
                Icon(Icons.Filled.MoreVert, contentDescription = "Item actions", tint = MaterialTheme.colorScheme.onSurfaceVariant)
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
