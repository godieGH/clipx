package com.godiegh.clipx.ui

import android.widget.Toast
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import com.godiegh.clipx.ClipxApplication
import com.godiegh.clipx.ClipxCoreViewModel
import com.godiegh.clipx.AndroidClipboardPlatform
import com.godiegh.clipx.ui.theme.ClipxLiveBackground

private enum class AppRoute {
    DEVICES,
    HISTORY,
    PAIR_NEW,
    THIS_DEVICE,
    DEVICE_DETAILS,
    CANT_SEE_DEVICE,
}

enum class RootTab { DEVICES, HISTORY }

@Composable
fun ClipxApp(viewModel: ClipxCoreViewModel) {
    var tab by remember { mutableStateOf(RootTab.DEVICES) }
    var route by remember { mutableStateOf(AppRoute.DEVICES) }
    var detailDeviceId by remember { mutableStateOf<String?>(null) }
    var selectedHistory by remember { mutableStateOf<ClipxHistoryItem?>(null) }
    var searchHistory by remember { mutableStateOf(false) }
    var forgetDeviceId by remember { mutableStateOf<String?>(null) }
    val context = LocalContext.current
    val application = context.applicationContext as ClipxApplication
    val clipxSheetController = application.sheetController

    fun openDevices() {
        viewModel.stopActiveScan()
        tab = RootTab.DEVICES
        route = AppRoute.DEVICES
    }

    fun openHistory() {
        viewModel.stopActiveScan()
        tab = RootTab.HISTORY
        route = AppRoute.HISTORY
    }

    LaunchedEffect(route) {
        if (route == AppRoute.PAIR_NEW) viewModel.startActiveScan()
        else viewModel.stopActiveScan()
    }

    LaunchedEffect(Unit) {
        viewModel.messages.collect { message ->
            Toast.makeText(context, message, Toast.LENGTH_SHORT).show()
        }
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
                            pairedDevices = viewModel.pairedDevices,
                            availableDevices = viewModel.availableDevices.map {
                                AvailableDeviceUi(it, viewModel.requestingPairId == it.id)
                            },
                            onOpenPair = { route = AppRoute.PAIR_NEW },
                            onOpenThisDevice = { route = AppRoute.THIS_DEVICE },
                            onOpenDetails = { id ->
                                detailDeviceId = id
                                route = AppRoute.DEVICE_DETAILS
                            },
                            onPair = viewModel::pair,
                            onConnect = viewModel::connect,
                        )

                        AppRoute.DEVICE_DETAILS -> {
                            val device = viewModel.pairedDevices.firstOrNull { it.id == detailDeviceId }
                            DeviceDetailsScreen(
                                device = device,
                                onBack = ::openDevices,
                                onAutoConnectChange = { checked ->
                                    detailDeviceId?.let { viewModel.setAutoConnect(it, checked) }
                                },
                                onDisconnect = {
                                    detailDeviceId?.let { viewModel.disconnect(it) }
                                },
                                onForget = {
                                    forgetDeviceId = detailDeviceId
                                },
                            )
                        }

                        AppRoute.PAIR_NEW -> PairNewDeviceScreen(
                            availableDevices = viewModel.availableDevices.map {
                                AvailableDeviceUi(it, viewModel.requestingPairId == it.id)
                            },
                            onBack = ::openDevices,
                            requestId = viewModel.requestingPairId,
                            onPair = viewModel::pair,
                            onCantSeeDevice = { route = AppRoute.CANT_SEE_DEVICE },
                        )

                        AppRoute.THIS_DEVICE -> ThisDeviceScreen(
                            identity = viewModel.identity,
                            onBack = ::openDevices,
                        )

                        AppRoute.CANT_SEE_DEVICE -> CantSeeDeviceScreen(onBack = { route = AppRoute.PAIR_NEW })

                        AppRoute.HISTORY -> HistoryScreen(
                            items = viewModel.historyItems,
                            search = searchHistory,
                            onSearchChange = { searchHistory = it },
                            onBack = ::openHistory,
                            onClearAll = viewModel::clearHistory,
                            onItemActions = { selectedHistory = it },
                            onCopyItem = { item ->
                                AndroidClipboardPlatform.copyWithoutSync(context, item.content)
                                Toast.makeText(context, "Copied to clipboard", Toast.LENGTH_SHORT).show()
                            },
                        )
                    }
                }
            }

            selectedHistory?.let { item ->
                HistoryActionsSheet(
                    item = item,
                    onDismiss = { selectedHistory = null },
                    onCopy = {
                        AndroidClipboardPlatform.copyWithoutSync(context, item.content)
                        Toast.makeText(context, "Copied to clipboard", Toast.LENGTH_SHORT).show()
                        selectedHistory = null
                    },
                    onRemove = {
                        viewModel.removeHistory(item.id)
                        selectedHistory = null
                    },
                )
            }

            forgetDeviceId?.let { deviceId ->
                AlertDialog(
                    onDismissRequest = { forgetDeviceId = null },
                    title = { Text("Forget device?") },
                    text = { Text("This removes the trusted device from Clipx. You will need to pair again to use it.") },
                    confirmButton = {
                        TextButton(
                            onClick = {
                                forgetDeviceId = null
                                viewModel.forget(deviceId)
                                openDevices()
                            },
                        ) { Text("Forget") }
                    },
                    dismissButton = {
                        TextButton(onClick = { forgetDeviceId = null }) { Text("Cancel") }
                    },
                )
            }

            ClipxDecisionSheetHost(clipxSheetController)
        }
    }
}
