package com.godiegh.clipx

import android.app.Application
import com.godiegh.clipx.ffi.initLogging
import com.godiegh.clipx.ui.ClipxSheetController

class ClipxApplication : Application() {
    val sheetController = ClipxSheetController()

    override fun onCreate() {
        super.onCreate()
        initLogging()
        ClipxCoreForegroundService.start(this)
    }
}
