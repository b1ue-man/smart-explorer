package app.smartexplorer.android.launcher.se

/**
 * The key of a device in the documents provider (`devices/<key>`, U40-14), the same rule as `mount::device_key_of` in
 * the core: `direct~<contact>` or `room~<profile>~<device>`, every byte of the UTF-8 form outside `[A-Za-z0-9._-]` as
 * `%` and two capital hex digits. The core stays the only truth: when it changes, the vectors of the test change too.
 */
object DeviceKey {
    private const val DIRECT_PREFIX = "share://direct/"
    private const val ROOM_PREFIX = "share://room/"
    private const val HEX = "0123456789ABCDEF"

    fun direct(contactId: String): String = "direct~" + escape(contactId)

    fun room(profileId: String, deviceId: String): String = "room~" + escape(profileId) + "~" + escape(deviceId)

    /** The key of a `location` of the list (`share://direct/<id>` or `share://room/<profile>/<device>`), else `null`. */
    fun of(location: String): String? = when {
        location.startsWith(DIRECT_PREFIX) -> location.removePrefix(DIRECT_PREFIX).substringBefore('/').takeIf { it.isNotEmpty() }?.let { direct(it) }
        location.startsWith(ROOM_PREFIX) -> {
            val parts = location.removePrefix(ROOM_PREFIX).split('/')
            if (parts.size >= 2 && parts[0].isNotEmpty() && parts[1].isNotEmpty()) room(parts[0], parts[1]) else null
        }
        else -> null
    }

    private fun escape(text: String): String {
        val out = StringBuilder()
        for (byte in text.toByteArray(Charsets.UTF_8)) {
            val b = byte.toInt() and 0xFF
            val c = b.toChar()
            if (c in 'A'..'Z' || c in 'a'..'z' || c in '0'..'9' || c == '.' || c == '_' || c == '-') {
                out.append(c)
            } else {
                out.append('%').append(HEX[b shr 4]).append(HEX[b and 0x0F])
            }
        }
        return out.toString()
    }
}
