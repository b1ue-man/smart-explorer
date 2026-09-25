pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

// The Android half of rustls-platform-verifier ships as a Maven repository inside the
// `rustls-platform-verifier-android` crate. CI resolves that directory with
// `cargo metadata --filter-platform aarch64-linux-android` and passes it as
// `-PrustlsVerifierMaven=<dir>` (absolute, or relative to this directory).
val rustlsVerifierMaven: java.io.File = run {
    val raw = providers.gradleProperty("rustlsVerifierMaven").orNull?.trim()
    if (raw.isNullOrEmpty()) {
        throw GradleException(
            "Gradle property 'rustlsVerifierMaven' is missing. Pass " +
                "-PrustlsVerifierMaven=<rustls-platform-verifier-android crate>/maven " +
                "(resolve it with: cargo metadata --format-version 1 " +
                "--filter-platform aarch64-linux-android --manifest-path native/Cargo.toml)."
        )
    }
    val candidate = java.io.File(raw)
    val dir = if (candidate.isAbsolute) candidate else java.io.File(settingsDir, raw)
    if (!java.io.File(dir, "rustls/rustls-platform-verifier").isDirectory) {
        throw GradleException(
            "Gradle property 'rustlsVerifierMaven' points to '${dir.path}', which does not " +
                "contain the Maven module rustls/rustls-platform-verifier."
        )
    }
    dir
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
        exclusiveContent {
            forRepository {
                maven {
                    name = "rustlsPlatformVerifier"
                    url = rustlsVerifierMaven.toURI()
                }
            }
            filter {
                includeGroup("rustls")
            }
        }
    }
}

rootProject.name = "SmartExplorer"
include(":app")
