package com.godiegh.clipx

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class WifiDirectLinkLogicTest {
    @Test
    fun directCredentialsAreOrderIndependent() {
        val a = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        val b = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"

        val left = deriveDirectCredentials(a, b)
        val right = deriveDirectCredentials(b, a)

        assertEquals(left, right)
        assertEquals("DIRECT-cx-", left.first.substring(0, 10))
        assertEquals(32, left.second.length)
    }

    @Test
    fun hostSelectionIsDeterministic() {
        assertEquals(true, shouldHostPeer("ffff", "aaaa"))
        assertEquals(false, shouldHostPeer("aaaa", "ffff"))
        assertEquals(false, shouldHostPeer("abcd", "abcd"))
    }

    @Test
    fun invalidDeviceAddressesAreRejected() {
        assertNull(normalizeDeviceAddress(""))
        assertNull(normalizeDeviceAddress("not-a-mac"))
        assertEquals("00:11:22:33:44:55", normalizeDeviceAddress("00:11:22:33:44:55"))
        assertEquals("00:aa:bb:33:44:55", normalizeDeviceAddress("00-AA-bb-33-44-55"))
    }
}
