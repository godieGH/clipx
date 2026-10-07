package com.godiegh.clipx

import android.Manifest
import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.MacAddress
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.wifi.WifiManager
import android.net.wifi.WifiNetworkSpecifier
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pDevice
import android.net.wifi.p2p.WifiP2pManager
import android.net.wifi.p2p.nsd.WifiP2pDnsSdServiceInfo
import android.net.wifi.p2p.nsd.WifiP2pDnsSdServiceRequest
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import java.security.MessageDigest
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * Wi-Fi Direct is used ONLY as an extra network link. It never carries ClipX
 * traffic itself and never tells core about peers: once the P2P group exists,
 * core's normal UDP discovery + WebSocket transport run over it unchanged.
 *
 * Flow: advertise a DNS-SD service carrying our fingerprint, discover others,
 * and for each ClipX peer found, the device with the higher fingerprint hosts
 * a group while the other joins it. Group name/passphrase are derived from
 * both fingerprints, so joining needs no user prompt (API 29+).
 */
@SuppressLint("MissingPermission")
class WifiDirectLink(
    context: Context,
    private val fingerprint: String,
    private val deviceName: String,
    private val wsPort: Int,
    /** True when core currently sees any peer; Windows hosts are only joined when false. */
    private val hasPeers: suspend () -> Boolean,
) {
    private val ctx = context.applicationContext
    private val manager = ctx.getSystemService(Context.WIFI_P2P_SERVICE) as? WifiP2pManager
    private val handler = Handler(Looper.getMainLooper())
    private var channel: WifiP2pManager.Channel? = null
    private var running = false
    private var inGroup = false
    private var busy = false
    private var attempts = 0
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val wifi = ctx.getSystemService(Context.WIFI_SERVICE) as WifiManager
    private val cm = ctx.getSystemService(ConnectivityManager::class.java)
    private var specifierCallback: ConnectivityManager.NetworkCallback? = null
    private var lastWindowsJoin = 0L

    private val scanReceiver = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) = onScanResults()
    }

    private val stateReceiver = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            val m = manager ?: return
            val ch = channel ?: return
            m.requestGroupInfo(ch) { group ->
                inGroup = group != null
                if (inGroup) { busy = false; attempts = 0 }
            }
        }
    }

    private val rediscover = object : Runnable {
        override fun run() {
            if (!running) return
            if (!inGroup) discover()
            scanForWindowsHost()
            handler.postDelayed(this, REDISCOVER_MS)
        }
    }

    /** Idempotent. Returns false if permission/hardware is missing. */
    fun start(): Boolean {
        if (running) return true
        val m = manager ?: return false
        if (!hasPermission()) { Log.w(TAG, "start: permission missing"); return false }
        channel = m.initialize(ctx, Looper.getMainLooper(), null) ?: return false
        running = true
        ctx.registerReceiver(
            stateReceiver,
            IntentFilter(WifiP2pManager.WIFI_P2P_CONNECTION_CHANGED_ACTION),
        )
        ctx.registerReceiver(
            scanReceiver,
            IntentFilter(WifiManager.SCAN_RESULTS_AVAILABLE_ACTION),
        )
        advertise()
        discover()
        scanForWindowsHost()
        handler.postDelayed(rediscover, REDISCOVER_MS)
        return true
    }

    fun stop() {
        if (!running) return
        running = false
        handler.removeCallbacksAndMessages(null)
        runCatching { ctx.unregisterReceiver(stateReceiver) }
        runCatching { ctx.unregisterReceiver(scanReceiver) }
        specifierCallback?.let {
            runCatching { cm.unregisterNetworkCallback(it) }
            cm.bindProcessToNetwork(null)
        }
        specifierCallback = null
        scope.cancel()
        val m = manager
        val ch = channel
        if (m != null && ch != null) {
            m.clearLocalServices(ch, null)
            m.clearServiceRequests(ch, null)
            m.stopPeerDiscovery(ch, null)
            m.removeGroup(ch, null)
        }
        channel = null
        inGroup = false
        busy = false
    }

    private fun hasPermission(): Boolean {
        val perm = if (Build.VERSION.SDK_INT >= 33) Manifest.permission.NEARBY_WIFI_DEVICES
        else Manifest.permission.ACCESS_FINE_LOCATION
        return ctx.checkSelfPermission(perm) == PackageManager.PERMISSION_GRANTED
    }

    private fun advertise() {
        val m = manager ?: return
        val ch = channel ?: return
        val txt = mapOf("fp" to fingerprint, "port" to wsPort.toString(), "name" to deviceName.take(32))
        val info = WifiP2pDnsSdServiceInfo.newInstance("clipx", SERVICE_TYPE, txt)
        m.clearLocalServices(ch, object : WifiP2pManager.ActionListener {
            override fun onSuccess() = m.addLocalService(ch, info, null)
            override fun onFailure(reason: Int) = m.addLocalService(ch, info, null)
        })
    }

    private fun discover() {
        val m = manager ?: return
        val ch = channel ?: return
        m.setDnsSdResponseListeners(
            ch,
            { _, _, _ -> },
            { domain, txt, device ->
                if (domain.contains(SERVICE_TYPE, ignoreCase = true)) {
                    txt["fp"]?.let { onPeer(it, device) }
                }
            },
        )
        val req = WifiP2pDnsSdServiceRequest.newInstance()
        m.clearServiceRequests(ch, object : WifiP2pManager.ActionListener {
            private fun next() = m.addServiceRequest(ch, req, object : WifiP2pManager.ActionListener {
                override fun onSuccess() = m.discoverPeers(ch, object : WifiP2pManager.ActionListener {
                    override fun onSuccess() = m.discoverServices(ch, null)
                    override fun onFailure(reason: Int) { Log.w(TAG, "discoverPeers failed: $reason") }
                })
                override fun onFailure(reason: Int) { Log.w(TAG, "addServiceRequest failed: $reason") }
            })
            override fun onSuccess() = next()
            override fun onFailure(reason: Int) = next()
        })
    }

    /**
     * Windows PCs can't be found through Android's P2P service discovery, so
     * they host a legacy group named DIRECT-cw-<fp8>. Only look for one when
     * core sees no peers, so a working network is never given up needlessly.
     */
    @Suppress("DEPRECATION")
    private fun scanForWindowsHost() {
        if (inGroup || specifierCallback != null) return
        scope.launch {
            if (!hasPeers()) handler.post { runCatching { wifi.startScan() } }
        }
    }

    @Suppress("DEPRECATION")
    private fun onScanResults() {
        if (!running || inGroup || specifierCallback != null) return
        if (System.currentTimeMillis() - lastWindowsJoin < WINDOWS_COOLDOWN_MS) return
        val results = runCatching { wifi.scanResults }
        Log.d(TAG, "scan results=${results.getOrNull()?.size} err=${results.exceptionOrNull()}")
        val ssid = results.getOrNull()
            ?.map { it.SSID }
            ?.firstOrNull { it.startsWith(WINDOWS_PREFIX) }
            ?: return
        Log.d(TAG, "found $ssid")
        scope.launch { if (!hasPeers()) handler.post { joinWindowsHost(ssid) } }
    }

    private fun joinWindowsHost(ssid: String) {
        if (specifierCallback != null) return
        lastWindowsJoin = System.currentTimeMillis()
        val host8 = ssid.removePrefix(WINDOWS_PREFIX)
        val pass = sha256Hex("clipx-p2p-ap|$host8").substring(0, 32)
        val spec = WifiNetworkSpecifier.Builder().setSsid(ssid).setWpa2Passphrase(pass).build()
        val req = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
            .removeCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .setNetworkSpecifier(spec)
            .build()
        val cb = object : ConnectivityManager.NetworkCallback() {
            // Route this process through the link so new sockets can reach the PC.
            override fun onAvailable(network: Network) { cm.bindProcessToNetwork(network) }
            override fun onLost(network: Network) {
                cm.bindProcessToNetwork(null)
                specifierCallback = null
            }
            override fun onUnavailable() { specifierCallback = null }
        }
        specifierCallback = cb
        cm.requestNetwork(req, cb, 30_000)
    }

    private fun sha256Hex(text: String): String =
        MessageDigest.getInstance("SHA-256").digest(text.toByteArray())
            .joinToString("") { "%02x".format(it) }

    private fun onPeer(peerFp: String, device: WifiP2pDevice) {
        if (!running || inGroup || busy || peerFp == fingerprint) return
        if (attempts >= MAX_ATTEMPTS) return
        busy = true
        attempts++
        val (name, pass) = credentials(peerFp)
        val config = WifiP2pConfig.Builder()
            .setNetworkName(name)
            .setPassphrase(pass)
            .apply {
                if (fingerprint > peerFp) {
                    // We host. Peer must be the joiner.
                } else {
                    setDeviceAddress(MacAddress.fromString(device.deviceAddress))
                }
            }
            .build()
        val m = manager ?: return
        val ch = channel ?: return
        val done = object : WifiP2pManager.ActionListener {
            override fun onSuccess() {
                // inGroup flips via stateReceiver; if nothing happens, unblock later.
                handler.postDelayed({ busy = false }, RETRY_MS)
            }
            override fun onFailure(reason: Int) {
                Log.w(TAG, "p2p group op failed: $reason")
                handler.postDelayed({ busy = false }, RETRY_MS)
            }
        }
        if (fingerprint > peerFp) m.createGroup(ch, config, done) else m.connect(ch, config, done)
    }

    /** Same result on both sides: derived from the two fingerprints, order-independent. */
    private fun credentials(peerFp: String): Pair<String, String> {
        val (a, b) = if (fingerprint < peerFp) fingerprint to peerFp else peerFp to fingerprint
        val h = sha256Hex("clipx-p2p|$a|$b")
        return "DIRECT-cx-${h.substring(0, 8)}" to h.substring(8, 40)
    }

    private companion object {
        const val TAG = "ClipxWifiDirect"
        const val SERVICE_TYPE = "_clipx._tcp"
        const val WINDOWS_PREFIX = "DIRECT-cw-"
        const val WINDOWS_COOLDOWN_MS = 5 * 60_000L
        const val REDISCOVER_MS = 30_000L
        const val RETRY_MS = 6_000L
        const val MAX_ATTEMPTS = 6
    }
}
