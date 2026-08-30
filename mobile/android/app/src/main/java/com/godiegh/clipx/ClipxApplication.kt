package com.godiegh.clipx

import android.app.Application
import com.godiegh.clipx.ffi.initLogging

class ClipxApplication: Application() {
    override fun onCreate() {
        super.onCreate()
        initLogging()
    }
}