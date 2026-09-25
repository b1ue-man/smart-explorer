import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

// The Rust workspace lives next to the Android project: <repo>/native and <repo>/android/app.
val cargoManifest = layout.projectDirectory.file("../../native/Cargo.toml")
val cargoLock = layout.projectDirectory.file("../../native/Cargo.lock")

/**
 * Reads `version = "x.y.z"` from the `[package]` table of native/Cargo.toml only, so an inline
 * dependency version elsewhere in the manifest can never match. The APK shares the desktop
 * release version; the release bump rewrites exactly this field.
 */
fun readCargoVersion(): Triple<Int, Int, Int> {
    val cargoToml = providers.fileContents(cargoManifest).asText.orNull
        ?: error("native/Cargo.toml not found or unreadable")
    val packageHeader = Regex("""(?m)^\[package\]\s*$""").find(cargoToml)
        ?: error("[package] table not found in native/Cargo.toml")
    val afterHeader = cargoToml.substring(packageHeader.range.last + 1)
    val nextHeader = Regex("""(?m)^\[.*\]\s*$""").find(afterHeader)
    val packageBody = if (nextHeader != null) afterHeader.substring(0, nextHeader.range.first) else afterHeader
    val versionLine = packageBody.lineSequence()
        .map { it.trim() }
        .firstOrNull { it.startsWith("version") && it.substringBefore("=").trim() == "version" }
        ?: error("version field not found inside [package] table of native/Cargo.toml")
    val versionValue = versionLine.substringAfter("=").trim().trim('"')
    val parts = versionValue.split(".").map { part ->
        part.trim().toIntOrNull() ?: error("Unexpected Cargo version format: '$versionValue'")
    }
    require(parts.size == 3) { "Unexpected Cargo version format: '$versionValue'" }
    require(parts[1] in 0..999 && parts[2] in 0..999) {
        "Minor and patch must stay below 1000 to keep versionCode monotonic: '$versionValue'"
    }
    return Triple(parts[0], parts[1], parts[2])
}

/**
 * The Maven artifact version of rustls-platform-verifier's Android component equals the version
 * of the `rustls-platform-verifier-android` crate in native/Cargo.lock; both must match exactly.
 */
fun readLockedCrateVersion(crate: String): String {
    val lines = (providers.fileContents(cargoLock).asText.orNull ?: error("native/Cargo.lock not found or unreadable"))
        .lines()
    val nameIndex = lines.indexOfFirst { it.trim() == "name = \"$crate\"" }
    require(nameIndex >= 0) { "$crate not found in native/Cargo.lock" }
    return lines.drop(nameIndex + 1)
        .firstOrNull { it.trimStart().startsWith("version = ") }
        ?.substringAfter('"', "")
        ?.substringBefore('"', "")
        ?.takeIf { it.isNotEmpty() }
        ?: error("version of $crate not found in native/Cargo.lock")
}

val (cargoMajor, cargoMinor, cargoPatch) = readCargoVersion()
val appVersionName = "$cargoMajor.$cargoMinor.$cargoPatch"
// 0.5.163 -> 5163: major * 1_000_000 + minor * 1_000 + patch (monotonic while minor/patch < 1000).
val appVersionCode = cargoMajor * 1_000_000 + cargoMinor * 1_000 + cargoPatch
val rustlsVerifierVersion = readLockedCrateVersion("rustls-platform-verifier-android")

// Release signing only from these four environment variables. Without all of them no release
// signing config exists and assembleRelease yields an unsigned APK, which the release job rejects.
val keystoreFile = providers.environmentVariable("ANDROID_KEYSTORE_FILE").orNull
val keystorePassword = providers.environmentVariable("ANDROID_KEYSTORE_PASSWORD").orNull
val releaseKeyAlias = providers.environmentVariable("ANDROID_KEY_ALIAS").orNull
val releaseKeyPassword = providers.environmentVariable("ANDROID_KEY_PASSWORD").orNull
val hasReleaseSigning = listOf(keystoreFile, keystorePassword, releaseKeyAlias, releaseKeyPassword)
    .all { !it.isNullOrBlank() }

android {
    namespace = "app.smartexplorer.android"
    compileSdk = 36

    defaultConfig {
        applicationId = "app.smartexplorer.android"
        minSdk = 30
        targetSdk = 36
        versionCode = appVersionCode
        versionName = appVersionName
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    signingConfigs {
        if (hasReleaseSigning) {
            create("release") {
                storeFile = file(keystoreFile!!)
                storePassword = keystorePassword
                keyAlias = releaseKeyAlias
                keyPassword = releaseKeyPassword
            }
        }
    }

    buildTypes {
        release {
            // R8 stays off: JNI entry points and the rustls verifier component are only reached
            // from native code. proguard-rules.pro keeps them should minification be enabled.
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            if (hasReleaseSigning) {
                signingConfig = signingConfigs.getByName("release")
            }
        }
    }

    packaging {
        jniLibs {
            useLegacyPackaging = true
        }
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    testOptions {
        animationsDisabled = true
    }

    lint {
        abortOnError = false
    }
}

kotlin {
    jvmToolchain(17)
    compilerOptions {
        jvmTarget = JvmTarget.JVM_17
    }
}

dependencies {
    implementation(platform(libs.compose.bom))
    androidTestImplementation(platform(libs.compose.bom))

    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.work.runtime.ktx)
    implementation(libs.androidx.material3.adaptive)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.kotlinx.coroutines.android)
    implementation("rustls:rustls-platform-verifier:$rustlsVerifierVersion")

    implementation(libs.compose.ui)
    implementation(libs.compose.ui.graphics)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons.core)
    debugImplementation(libs.compose.ui.tooling)
    debugImplementation(libs.compose.ui.test.manifest)

    testImplementation(libs.junit4)

    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(libs.androidx.test.espresso.core)
    androidTestImplementation(libs.androidx.test.espresso.intents)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.uiautomator)
    androidTestImplementation(libs.androidx.work.testing)
    androidTestImplementation(libs.compose.ui.test.junit4)
}
