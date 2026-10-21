package app.smartexplorer.android.ui.vaults

import app.smartexplorer.android.api.VaultTextPair
import app.smartexplorer.android.api.VaultTextRow
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** VLT-M11 (U31-11, U31-15, U31-16): texts of the vault screens and the merge of a text conflict. */
class VaultTextsTest {
    /** Every error code of C7.5 has a text of its own; no code falls through as itself. */
    @Test
    fun vault_task_android_texts_cover_codes() {
        val codes = VaultTexts.CODES
        assertEquals(
            listOf("vault_locked", "vault_use_provider", "vault_wrong_password", "vault_unsupported_format", "vault_hub", "vault_memory"),
            codes,
        )
        codes.forEach { code ->
            val text = VaultTexts.errorText(code, "Kerntext für $code", "Privat")
            assertTrue("$code ohne Text", text.isNotBlank())
            assertNotEquals("$code fällt auf den Code zurück", code, text)
        }
        assertEquals("Passwort falsch", VaultTexts.errorText("vault_wrong_password", "x"))
        assertEquals("Tresor Privat ist gesperrt. Entsperre ihn unter Mehr → Tresore.", VaultTexts.errorText("vault_locked", "x", "Privat"))
        assertEquals("Schlüsselableitung braucht 64 MiB, verfügbar 12 MiB", VaultTexts.errorText("vault_memory", "Schlüsselableitung braucht 64 MiB, verfügbar 12 MiB"))
        listOf("screen_off", "background", "idle", "manual", "process_end").forEach { reason ->
            assertTrue("Grund $reason", VaultTexts.reasonText(reason) != null)
        }
        assertEquals("Privat wurde gesperrt (Bildschirm aus).", VaultTexts.lockedNotice("Privat", "screen_off"))
    }

    /** The merged text keeps line endings and the final newline of version A (U31-9). */
    @Test
    fun vault_task_android_merge_assembles_choices() {
        val pair = VaultTextPair(
            a = "eins\r\nzwei\r\n",
            b = "eins\r\nZWEI\r\n",
            rows = listOf(VaultTextRow("eins", "eins", equal = true), VaultTextRow("zwei", "ZWEI", equal = false)),
            crlf = true,
            finalNewline = true,
        )
        assertEquals("eins\r\nzwei\r\n", assemble(pair, pair.rows.map(::defaultChoice)))
        assertEquals("eins\r\nzwei\r\nZWEI\r\n", assemble(pair, listOf(RowChoice(true, false), RowChoice(left = true, right = true))))
        assertEquals("eins\r\nZWEI\r\n", assemble(pair, listOf(RowChoice(true, false), RowChoice(left = false, right = true))))
        assertFalse(defaultChoice(VaultTextRow(null, "nur B", equal = false)).left)
    }
}
