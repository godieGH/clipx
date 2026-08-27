package com.godiegh.clipx.ui

import com.godiegh.clipx.AvailableDevice
import com.godiegh.clipx.ClipType
import com.godiegh.clipx.ConnectionStatus
import com.godiegh.clipx.DeviceType
import com.godiegh.clipx.PairedDevice
import com.godiegh.clipx.PairedStatus

object ClipxMockData {
    // IDs are intentionally represented in the same 64-hex-character form used by the real model.
    val pairedDevices = listOf(
        PairedDevice(
            id = "91AA22BB3C4D5E6F778899AABBCCDDEEFF00112233445566778899AABBCCDD00",
            name = "Meckz Laptop",
            type = DeviceType.WINDOWS,
            status = ConnectionStatus.CONNECTED,
            ipAddress = "192.168.1.33",
            wsPort = 8765,
            autoConnect = true,
        ),
        PairedDevice(
            id = "B4E24F6A809D1A2B3C4D5E6F708190A1B2C3D4E5F60718293A4B5C6D7E8F9012",
            name = "Office Linux",
            type = DeviceType.LINUX,
            status = ConnectionStatus.DISCONNECTED,
            ipAddress = "192.168.1.42",
            wsPort = 8765,
            autoConnect = false,
        ),
    )

    val availableDevices = listOf(
        AvailableDeviceUi(
            model = AvailableDevice("A1B2C3D4E5F60718293A4B5C6D7E8F90112233445566778899AABBCCDDEEFF00", "Home PC", DeviceType.WINDOWS, PairedStatus.IDLE),
        ),
        AvailableDeviceUi(
            model = AvailableDevice("112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00", "Dev MacBook", DeviceType.MACOS, PairedStatus.IDLE),
        ),
        AvailableDeviceUi(
            model = AvailableDevice("7F8E9A0B1C2D3E4F5061728394A5B6C7D8E9F00112233445566778899AABBCCD", "Living Room PC", DeviceType.WINDOWS, PairedStatus.IDLE),
        ),
    )

    val historyItems = listOf(
        ClipxHistoryItem("1", "const ClipX = () => { return \"Productivity\"; }", "const ClipX = () => { return \"Productivity\"; }", "Meckz Laptop", "just now", ClipType.TEXT),
        ClipxHistoryItem("2", "Design system colors #396CD8 #7B61FF #0E1117", "Design system colors #396CD8 #7B61FF #0E1117", "Office Linux", "2m ago", ClipType.TEXT),
        ClipxHistoryItem("3", "Ship the mobile pairing flow after the UI pass.", "Ship the mobile pairing flow after the UI pass.", "Meckz Laptop", "7m ago", ClipType.TEXT),
        ClipxHistoryItem("4", "192.168.1.42:8765", "192.168.1.42:8765", "Office Linux", "12m ago", ClipType.IMAGE),
    )
}
