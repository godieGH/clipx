package com.godiegh.clipx

import android.graphics.drawable.Icon
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService

class ClipxQuickSettingsTile : TileService() {

    override fun onStartListening() {
        super.onStartListening()
        updateTile()
    }

    override fun onClick() {
        super.onClick()

        val currentlyEnabled =
            ClipxCoreForegroundService.isBackgroundSyncEnabled(this)

        try {
            if (currentlyEnabled) {
                ClipxCoreForegroundService.disableBackgroundSync(this)
            } else {
                ClipxCoreForegroundService.enableBackgroundSync(this)
            }
        } catch (_: Throwable) {
            /*
             * If Android rejects a service start immediately, keep the tile
             * synchronized with the persisted preference rather than leaving
             * it showing the wrong state.
             */
        }

        updateTile()
    }

    private fun updateTile() {
        val tile = qsTile ?: return

        val enabled =
            ClipxCoreForegroundService.isBackgroundSyncEnabled(this)

        tile.icon = Icon.createWithResource(
            this,
            R.drawable.ic_notification_clipx,
        )

        tile.label = "ClipX Sync"

        tile.state = if (enabled) {
            Tile.STATE_ACTIVE
        } else {
            Tile.STATE_INACTIVE
        }

        tile.updateTile()
    }
}