package com.godiegh.clipx.ui

import android.content.ClipData
import android.content.Context
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import com.godiegh.clipx.ConnectionStatus
import com.godiegh.clipx.ui.theme.ClipxLiveBackground

private enum class AppRoute {
    DEVICES,
    HISTORY,
    PAIR_NEW,
    THIS_DEVICE,
    DEVICE_DETAILS,
}

enum class RootTab { DEVICES, HISTORY }

@Composable
fun ClipxApp() {
    var tab by remember { mutableStateOf(RootTab.DEVICES) }
    var route by remember { mutableStateOf(AppRoute.DEVICES) }
    var detailDeviceId by remember { mutableStateOf<String?>(null) }
    var pairedDevices by remember { mutableStateOf(ClipxMockData.pairedDevices) }
    var availableDevices by remember { mutableStateOf(ClipxMockData.availableDevices) }
    var historyItems by remember { mutableStateOf(ClipxMockData.historyItems) }
    var requestId by remember { mutableStateOf<String?>(null) }
    var selectedHistory by remember { mutableStateOf<ClipxHistoryItem?>(null) }
    var searchHistory by remember { mutableStateOf(false) }
    val context = LocalContext.current
    val clipxSheetController = remember { ClipxSheetController() }

    fun openDevices() {
        tab = RootTab.DEVICES
        route = AppRoute.DEVICES
    }

    fun openHistory() {
        tab = RootTab.HISTORY
        route = AppRoute.HISTORY
    }

    fun pair(id: String) {
        requestId = id
        availableDevices = availableDevices.map { device ->
            if (device.model.id == id) device.copy(requesting = true) else device
        }
    }

    fun copyToClipboard(item: ClipxHistoryItem) {
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("ClipX", item.content))
    }

    CompositionLocalProvider(LocalClipxSheetController provides clipxSheetController) {
        Box(modifier = Modifier.fillMaxSize()) {
            ClipxLiveBackground(modifier = Modifier.fillMaxSize())

            ClipxScaffold(
            currentTab = tab,
            showBottomBar = route == AppRoute.DEVICES || route == AppRoute.HISTORY,
                onDevices = ::openDevices,
                onHistory = ::openHistory,
            ) { innerPadding ->
            AnimatedContent(
                targetState = route,
                modifier = innerPadding,
                transitionSpec = { fadeIn() togetherWith fadeOut() },
                label = "clipx-route",
            ) { currentRoute ->
                when (currentRoute) {
                    AppRoute.DEVICES -> DevicesScreen(
                        pairedDevices = pairedDevices,
                        availableDevices = availableDevices,
                        onOpenPair = {
                            requestId = null
                            availableDevices = ClipxMockData.availableDevices
                            route = AppRoute.PAIR_NEW
                        },
                        onOpenThisDevice = { route = AppRoute.THIS_DEVICE },
                        onOpenDetails = { id ->
                            detailDeviceId = id
                            route = AppRoute.DEVICE_DETAILS
                        },
                        onPair = ::pair,
                        onConnect = { id ->
                            pairedDevices = pairedDevices.map { device ->
                                if (device.id == id) device.copy(status = ConnectionStatus.CONNECTED) else device
                            }
                        },
                    )

                    AppRoute.DEVICE_DETAILS -> {
                        val device = pairedDevices.firstOrNull { it.id == detailDeviceId }
                        DeviceDetailsScreen(
                            device = device,
                            onBack = ::openDevices,
                            onAutoConnectChange = { checked ->
                                if (detailDeviceId != null) {
                                    pairedDevices = pairedDevices.map { item ->
                                        if (item.id == detailDeviceId) item.copy(autoConnect = checked) else item
                                    }
                                }
                            },
                            onDisconnect = {
                                if (detailDeviceId != null) {
                                    pairedDevices = pairedDevices.map { item ->
                                        if (item.id == detailDeviceId) item.copy(status = ConnectionStatus.DISCONNECTED) else item
                                    }
                                }
                            },
                            onForget = {
                                if (detailDeviceId != null) {
                                    pairedDevices = pairedDevices.filterNot { it.id == detailDeviceId }
                                }
                                openDevices()
                            },
                        )
                    }

                    AppRoute.PAIR_NEW -> PairNewDeviceScreen(
                        availableDevices = availableDevices,
                        onBack = ::openDevices,
                        requestId = requestId,
                        onPair = ::pair,
                    )

                    AppRoute.THIS_DEVICE -> ThisDeviceScreen(onBack = ::openDevices)

                    AppRoute.HISTORY -> HistoryScreen(
                        items = historyItems,
                        search = searchHistory,
                        onSearchChange = { searchHistory = it },
                        onBack = ::openHistory,
                        onClearAll = { historyItems = emptyList() },
                        onItemActions = { selectedHistory = it },
                        onCopyItem = ::copyToClipboard,
                    )
                }
            }
        }

        selectedHistory?.let { item ->
            HistoryActionsSheet(
                item = item,
                onDismiss = { selectedHistory = null },
                onCopy = {
                    copyToClipboard(item)
                    selectedHistory = null
                },
                onRemove = {
                    historyItems = historyItems.filterNot { it.id == item.id }
                    selectedHistory = null
                },
            )
        }

            ClipxDecisionSheetHost(clipxSheetController)
        }
    }
}
