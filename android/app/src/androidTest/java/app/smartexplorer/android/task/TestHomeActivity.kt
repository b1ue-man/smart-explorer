package app.smartexplorer.android.task

import androidx.activity.ComponentActivity

/**
 * An empty home screen of the test APK (U35-33): only its HOME filter matters, so the system knows two
 * launchers while it is switched on. Declared in `src/androidTest/AndroidManifest.xml`, off by default;
 * `LauncherTaskTest.setTestHome` switches it, with the permission `CHANGE_COMPONENT_ENABLED_STATE` that the
 * shell lends to the test process (the shell itself may not change the components of this APK).
 */
class TestHomeActivity : ComponentActivity()
