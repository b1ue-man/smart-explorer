package app.smartexplorer.android.core

import android.app.Application
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Log
import java.io.File
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Data key of the core's credential store (`secretsKey` of api.md §1).
 *
 * The 32-byte key lives only wrapped in `noBackupFilesDir/secrets-key-v2.bin`:
 * AES-256-GCM under a non-exportable Android Keystore key (TEE or StrongBox
 * where the device has one). No user authentication is required, so the
 * background service can open the store after the first unlock, exactly when
 * the credential-encrypted app storage itself becomes available.
 *
 * An existing wrapped key is never replaced: if the Keystore cannot unwrap it,
 * no key is handed in and the core keeps the sealed records closed instead of
 * silently starting over with a new key.
 */
internal object SecretsKey {
    private const val TAG = "SecretsKey"
    private const val KEYSTORE = "AndroidKeyStore"
    private const val ALIAS = "smart_explorer_secrets_v2"
    private const val FILE_NAME = "secrets-key-v2.bin"
    private const val KEY_BYTES = 32
    private const val IV_BYTES = 12
    private const val TAG_BITS = 128
    private const val TRANSFORMATION = "AES/GCM/NoPadding"
    private const val ATTEMPTS = 3
    private const val RETRY_DELAY_MS = 250L

    /**
     * The data key as lowercase hex, or null when the Keystore failed. A failure is retried
     * briefly: a transient hiccup right after boot must not leave a first-start store with
     * the weaker key-file protector, nor lock an existing store for the life of the process.
     * Nothing logged here contains key material.
     */
    fun hex(app: Application): String? {
        for (attempt in 1..ATTEMPTS) {
            try {
                val key = load(File(app.noBackupFilesDir, FILE_NAME))
                try {
                    return key.joinToString("") { "%02x".format(it.toInt() and 0xff) }
                } finally {
                    key.fill(0)
                }
            } catch (e: Exception) {
                Log.e(
                    TAG,
                    "credential data key unavailable (attempt $attempt of $ATTEMPTS): " +
                        "${e.javaClass.simpleName}: ${e.message}",
                )
            }
            if (attempt < ATTEMPTS && !pause()) return null
        }
        return null
    }

    /** Waits before the next attempt; false when the thread was interrupted. */
    private fun pause(): Boolean =
        try {
            Thread.sleep(RETRY_DELAY_MS)
            true
        } catch (interrupted: InterruptedException) {
            Thread.currentThread().interrupt()
            false
        }

    private fun load(file: File): ByteArray {
        val wrapping = wrappingKey(createIfMissing = !file.exists())
        if (file.exists()) {
            return unwrap(wrapping, file.readBytes())
        }
        val key = ByteArray(KEY_BYTES).also { SecureRandom().nextBytes(it) }
        val wrapped = wrap(wrapping, key)
        val stage = File(file.parentFile, "$FILE_NAME.tmp")
        stage.outputStream().use { out ->
            out.write(wrapped)
            out.fd.sync()
        }
        if (!stage.renameTo(file)) {
            stage.delete()
            key.fill(0)
            throw IllegalStateException("wrapped data key could not be stored")
        }
        // Read back through the Keystore before the key is ever used.
        val reread = unwrap(wrapping, file.readBytes())
        val same = reread.contentEquals(key)
        reread.fill(0)
        if (!same) {
            key.fill(0)
            throw IllegalStateException("wrapped data key does not open again")
        }
        return key
    }

    private fun wrappingKey(createIfMissing: Boolean): SecretKey {
        val store = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (store.getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
        check(createIfMissing) { "Keystore key is missing for an existing wrapped data key" }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(
                ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .setRandomizedEncryptionRequired(true)
                .build(),
        )
        return generator.generateKey()
    }

    private fun wrap(wrapping: SecretKey, key: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, wrapping)
        val iv = cipher.iv
        check(iv.size == IV_BYTES) { "unexpected GCM IV length ${iv.size}" }
        return iv + cipher.doFinal(key)
    }

    private fun unwrap(wrapping: SecretKey, blob: ByteArray): ByteArray {
        check(blob.size == IV_BYTES + KEY_BYTES + TAG_BITS / 8) { "wrapped data key has ${blob.size} bytes" }
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, wrapping, GCMParameterSpec(TAG_BITS, blob, 0, IV_BYTES))
        val key = cipher.doFinal(blob, IV_BYTES, blob.size - IV_BYTES)
        check(key.size == KEY_BYTES) { "unwrapped data key has ${key.size} bytes" }
        return key
    }
}
