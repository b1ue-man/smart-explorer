package app.smartexplorer.android.ui.vaults

import android.app.KeyguardManager
import android.content.Context
import android.hardware.biometrics.BiometricManager.Authenticators
import android.hardware.biometrics.BiometricPrompt
import android.os.CancellationSignal
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import java.security.KeyStore
import java.util.Base64
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume

/**
 * „Passwort merken“ on Android (U31, answer S1 of the security agent): the core never stores a
 * vault password in cleartext. The app seals it with an AES-256-GCM key of the Android Keystore
 * that needs the device lock (biometrics of class 3 or PIN/pattern/password) for every single use,
 * bound to the site ID as associated data; the core keeps only the envelope `android-kw1:…`.
 * Only a [Cipher] the user confirmed through [BiometricPrompt] can seal or open it.
 */
internal object VaultSavedPassword {
    private const val KEYSTORE = "AndroidKeyStore"
    private const val ALIAS = "smart_explorer_vault_saved_v1"
    private const val PREFIX = "android-kw1:"
    private const val TRANSFORMATION = "AES/GCM/NoPadding"
    private const val IV_BYTES = 12
    private const val TAG_BITS = 128

    /** The key needs a device lock; without one the check box stays off (S1). */
    fun available(context: Context): Boolean =
        context.getSystemService(KeyguardManager::class.java)?.isDeviceSecure == true

    /** A cipher to seal a password; throws when the Keystore refuses the key. */
    fun sealCipher(): Cipher {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        try {
            cipher.init(Cipher.ENCRYPT_MODE, key(create = true))
        } catch (e: KeyPermanentlyInvalidatedException) {
            // The device lock changed: envelopes of the old key cannot open any more anyway.
            deleteKey()
            cipher.init(Cipher.ENCRYPT_MODE, key(create = true))
        }
        return cipher
    }

    /**
     * A cipher to open [sealed]; `null` when the envelope can no longer open: no prefix, a changed
     * device lock ([KeyPermanentlyInvalidatedException]) or no key. The caller then forgets it.
     */
    fun openCipher(sealed: String): Cipher? {
        val blob = decode(sealed) ?: return null
        return try {
            Cipher.getInstance(TRANSFORMATION).apply {
                init(Cipher.DECRYPT_MODE, key(create = false) ?: return null, GCMParameterSpec(TAG_BITS, blob, 0, IV_BYTES))
            }
        } catch (e: KeyPermanentlyInvalidatedException) {
            deleteKey()
            null
        }
    }

    fun seal(cipher: Cipher, site: String, password: String): String {
        cipher.updateAAD(site.toByteArray(Charsets.UTF_8))
        val sealed = cipher.iv + cipher.doFinal(password.toByteArray(Charsets.UTF_8))
        return PREFIX + Base64.getUrlEncoder().withoutPadding().encodeToString(sealed)
    }

    /** The password inside [sealed]; `null` when the tag does not match (another site, damage). */
    fun open(cipher: Cipher, site: String, sealed: String): String? {
        val blob = decode(sealed) ?: return null
        cipher.updateAAD(site.toByteArray(Charsets.UTF_8))
        return try {
            String(cipher.doFinal(blob, IV_BYTES, blob.size - IV_BYTES), Charsets.UTF_8)
        } catch (e: AEADBadTagException) {
            null
        }
    }

    /** Outcome of the device lock prompt. */
    sealed interface Confirmed {
        data class Ok(val cipher: Cipher) : Confirmed

        /** The user closed the prompt: no message (S1). */
        data object Dismissed : Confirmed

        data class Failed(val text: String) : Confirmed
    }

    /** Asks for the device lock with [cipher] as crypto object (framework API, no Gradle dependency). */
    suspend fun confirm(context: Context, title: String, subtitle: String, cipher: Cipher): Confirmed =
        suspendCancellableCoroutine { continuation ->
            val signal = CancellationSignal()
            continuation.invokeOnCancellation { signal.cancel() }
            val prompt = BiometricPrompt.Builder(context)
                .setTitle(title)
                .setSubtitle(subtitle)
                .setAllowedAuthenticators(Authenticators.BIOMETRIC_STRONG or Authenticators.DEVICE_CREDENTIAL)
                .build()
            prompt.authenticate(
                BiometricPrompt.CryptoObject(cipher),
                signal,
                context.mainExecutor,
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        val confirmed = result.cryptoObject?.cipher
                        if (continuation.isActive) {
                            continuation.resume(confirmed?.let { Confirmed.Ok(it) } ?: Confirmed.Failed("Keine Bestätigung"))
                        }
                    }

                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        if (!continuation.isActive) return
                        val dismissed = errorCode == BiometricPrompt.BIOMETRIC_ERROR_USER_CANCELED ||
                            errorCode == BiometricPrompt.BIOMETRIC_ERROR_CANCELED ||
                            errorCode == BiometricPrompt.BIOMETRIC_ERROR_NEGATIVE_BUTTON
                        continuation.resume(if (dismissed) Confirmed.Dismissed else Confirmed.Failed("Bestätigung fehlgeschlagen: $errString"))
                    }
                },
            )
        }

    private fun decode(sealed: String): ByteArray? {
        if (!sealed.startsWith(PREFIX)) return null
        val blob = try {
            Base64.getUrlDecoder().decode(sealed.removePrefix(PREFIX))
        } catch (e: IllegalArgumentException) {
            return null
        }
        return blob.takeIf { it.size > IV_BYTES + TAG_BITS / 8 }
    }

    private fun store(): KeyStore = KeyStore.getInstance(KEYSTORE).apply { load(null) }

    private fun key(create: Boolean): SecretKey? {
        (store().getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
        if (!create) return null
        return try {
            generate(strongBox = true)
        } catch (e: StrongBoxUnavailableException) {
            generate(strongBox = false)
        }
    }

    private fun generate(strongBox: Boolean): SecretKey {
        val spec = KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setUserAuthenticationRequired(true)
            .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL)
            .setUnlockedDeviceRequired(true)
            .setInvalidatedByBiometricEnrollment(false)
            .setIsStrongBoxBacked(strongBox)
            .build()
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE).run {
            init(spec)
            generateKey()
        }
    }

    private fun deleteKey() {
        try {
            store().deleteEntry(ALIAS)
        } catch (e: java.security.KeyStoreException) {
            // Nothing left to delete.
        }
    }
}
