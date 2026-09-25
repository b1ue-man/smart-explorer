package app.smartexplorer.android.ui.share

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** PIN warning of the discoverable dialog: short, repeated and straight-run PINs are weak. */
class WeakPinTest {
    @Test
    fun weakPins() {
        for (pin in listOf("", "123", "1111", "1234", "4321", "98765", "aaaa")) {
            assertTrue("\"$pin\" sollte schwach sein", isWeakPin(pin))
        }
    }

    @Test
    fun acceptablePins() {
        for (pin in listOf("4826", "1357", "0192", "ab12", "13579")) {
            assertFalse("\"$pin\" sollte nicht schwach sein", isWeakPin(pin))
        }
    }
}
