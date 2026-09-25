# Minification is disabled for release builds (app/build.gradle.kts). These rules keep the
# classes that only native code reaches, so enabling R8 later cannot break JNI resolution.

# JNI entry points: Java_app_smartexplorer_android_core_NativeBridge_{init,call,pollEvents}
-keep class app.smartexplorer.android.core.NativeBridge {
    native <methods>;
}

# rustls-platform-verifier Android component (called from Rust over JNI)
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }
