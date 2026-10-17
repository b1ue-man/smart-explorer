package app.smartexplorer.android.task

import android.app.NotificationManager
import android.os.Build
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.api.ACCESS_ALLOW
import app.smartexplorer.android.api.AccessAccount
import app.smartexplorer.android.api.AccessApproval
import app.smartexplorer.android.api.AccessChange
import app.smartexplorer.android.api.AccessOverview
import app.smartexplorer.android.api.AccessProviderInfo
import app.smartexplorer.android.api.AccessTargetInfo
import app.smartexplorer.android.api.ApprovalCommandInfo
import app.smartexplorer.android.api.CAP_COMMANDS
import app.smartexplorer.android.api.CAP_SCREEN_VIEW
import app.smartexplorer.android.api.CAP_TERMINAL
import app.smartexplorer.android.api.CapabilityRow
import app.smartexplorer.android.api.DECISION_ALWAYS
import app.smartexplorer.android.api.DECISION_DENY
import app.smartexplorer.android.api.DECISION_ONCE
import app.smartexplorer.android.api.ExecProviderInfo
import app.smartexplorer.android.api.ExecTarget
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.api.ValidUntil
import app.smartexplorer.android.api.setAccessArgs
import app.smartexplorer.android.service.ApprovalDelivery
import app.smartexplorer.android.service.approvalAction
import app.smartexplorer.android.service.approvalNotification
import app.smartexplorer.android.service.approvalUri
import app.smartexplorer.android.service.deliverApproval
import app.smartexplorer.android.system.Notifications
import app.smartexplorer.android.ui.share.ExecHostActions
import app.smartexplorer.android.ui.share.ExecHostSection
import app.smartexplorer.android.ui.theme.SmartExplorerTheme
import kotlinx.serialization.json.JsonNull
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * PRM-M10 on the device (plan 2026-10-17 „Rechte, se Desktop, Tresore“, strang-rechte.md R 401):
 * the question channel with its actions, the report of a question the app can show nowhere, and the
 * access dialog that allows a device with two actions. The bridge signals are Rust host tests
 * (`share_access_tests.rs`); `perm_task_android_screen_approval_actions_are_activity_intents`
 * follows with SCR-M8 (U27). No second device takes part: the question itself is built here.
 */
@RunWith(AndroidJUnit4::class)
class PermTaskTest {
    @get:Rule
    val compose = createComposeRule()

    private val question = AccessApproval(
        id = "0123456789abcdef0123456789abcdef",
        deviceName = "Laptop",
        deviceId = "d-laptop",
        fingerprint = "fp-laptop-0123456789",
        capability = CAP_COMMANDS,
        capabilityLabel = "Befehle",
        text = "Laptop möchte Befehle ausführen",
        accountLabel = "u0_a1",
        command = ApprovalCommandInfo(mode = "shell", text = "uptime"),
        surface = "notification",
    )

    @Test
    fun perm_task_android_approval_channel_high_importance() {
        Notifications.ensureChannels(appContext)
        val manager = appContext.getSystemService(NotificationManager::class.java)
        val channel = manager.getNotificationChannel(Notifications.CHANNEL_ACCESS_APPROVAL)
        assertNotNull("Kanal ${Notifications.CHANNEL_ACCESS_APPROVAL} fehlt", channel)
        assertEquals(NotificationManager.IMPORTANCE_HIGH, channel.importance)
        TaskReport.note("access_approval", "importance=${channel.importance}")

        // Exec question: answers as broadcasts; allowing needs an unlocked device (API 31+), on
        // Android 11 it is left to the dialog in the app.
        val exec = approvalNotification(appContext, question, Notifications.ID_APPROVAL_FIRST)
        assertEquals(Notifications.CHANNEL_ACCESS_APPROVAL, exec.channelId)
        val titles = exec.actions.orEmpty().map { it.title.toString() }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            assertEquals(listOf("Diesmal erlauben", "Immer erlauben", "Ablehnen"), titles)
            assertTrue(exec.actions[0].isAuthenticationRequired)
            assertTrue(exec.actions[1].isAuthenticationRequired)
        } else {
            assertEquals(listOf("Ablehnen"), titles)
        }
        assertNotNull("Wischen muss ablehnen", exec.deleteIntent)
        assertNotNull("Sperrbildschirm braucht die öffentliche Fassung", exec.publicVersion)
        assertTrue("Sperrbildschirm zeigt Aktionen", exec.publicVersion.actions.isNullOrEmpty())

        // Screen question before SCR-M8 (W1): only „Ablehnen“.
        val screen = approvalNotification(appContext, question.copy(capability = CAP_SCREEN_VIEW), Notifications.ID_APPROVAL_FIRST + 1)
        assertEquals(listOf("Ablehnen"), screen.actions.orEmpty().map { it.title.toString() })

        // Every action names exactly its question and answer.
        for (decision in listOf(DECISION_ONCE, DECISION_ALWAYS, DECISION_DENY)) {
            assertEquals(question.id to decision, approvalAction(approvalUri(question.id, decision)))
        }
        assertNull(approvalAction(approvalUri("xyz", DECISION_ONCE)))
        assertNull(approvalAction(approvalUri(question.id, "vielleicht")))
        assertNull(approvalAction(null))
    }

    @Test
    fun perm_task_android_undeliverable_reports_unavailable() = coreTest {
        val reported = mutableListOf<String>()
        // Hidden app, no notification possible: reported, the other device gets approval_unavailable.
        assertEquals(
            ApprovalDelivery.Undeliverable,
            deliverApproval(question, visible = false, showInApp = { false }, post = { false }) { reported += it },
        )
        assertEquals(listOf(question.id), reported)
        // Visible app: a dialog in the app, nothing reported, even without notifications.
        assertEquals(
            ApprovalDelivery.InApp,
            deliverApproval(question, visible = true, showInApp = { true }, post = { false }) { reported += it },
        )
        assertEquals(
            ApprovalDelivery.InApp,
            deliverApproval(question, visible = true, showInApp = { false }, post = { false }) { reported += it },
        )
        // Hidden app with notifications: a notification, nothing reported.
        assertEquals(
            ApprovalDelivery.Notification,
            deliverApproval(question, visible = false, showInApp = { false }, post = { true }) { reported += it },
        )
        assertEquals(1, reported.size)

        // The bridge: a question that is not open is not_found, a malformed id or answer invalid.
        Api.obj("bg.ensureDaemon")
        val unknown = "ab".repeat(16)
        Api.failure("share.reportApprovalUndeliverable", args("approvalId" to unknown), "not_found")
        Api.failure("share.reportApprovalUndeliverable", args("approvalId" to "x"), "invalid")
        Api.failure("share.decideAccessApproval", args("approvalId" to unknown, "decision" to DECISION_ONCE), "not_found")
        Api.failure("share.decideAccessApproval", args("approvalId" to unknown, "decision" to "vielleicht"), "invalid")
        val pending = Api.obj("share.accessApprovals").array("pending")
        TaskReport.note("share.accessApprovals", pending.toString())
    }

    @Test
    fun perm_task_android_access_dialog_two_actions() {
        val target = ExecTarget(
            targetKey = "direct/d-laptop/fp-laptop",
            deviceId = "d-laptop",
            name = "Laptop",
            fingerprint = "fp-laptop",
            baseAuthorized = true,
        )
        val status = ShareStatus(
            execProvider = ExecProviderInfo(available = true, provider = "android-subreaper", userLabel = "u0_a1"),
            execTargets = listOf(target),
        )
        val overview = AccessOverview(
            execProvider = AccessProviderInfo(
                userLabel = "u0_a1",
                otherAccountsNote = "App-Konto von Smart Explorer.",
                capabilities = listOf(
                    CapabilityRow(CAP_COMMANDS, "Befehle", "Programme und Shell-Befehle starten", usesAccount = true),
                    CapabilityRow(CAP_TERMINAL, "Terminal", "Ein Terminal mit der Login-Shell öffnen", usesAccount = true),
                ),
            ),
            execTargets = listOf(AccessTargetInfo(target.targetKey)),
        )
        val saved = mutableListOf<Pair<ExecTarget, AccessChange>>()
        val actions = ExecHostActions(
            allow = {},
            revoke = {},
            stop = {},
            programs = {},
            log = {},
            setAccess = { savedTarget, change -> saved += savedTarget to change },
        )
        compose.setContent {
            SmartExplorerTheme {
                Column(Modifier.verticalScroll(rememberScrollState())) { ExecHostSection(status, null, actions, overview) }
            }
        }
        // 1: „Erlauben…“ at the device, 2: the button that names the consequence.
        compose.onNodeWithText("Erlauben…").performClick()
        compose.onNodeWithText("Als App-Konto erlauben").performClick()
        compose.runOnIdle {
            assertEquals(1, saved.size)
            val (savedTarget, change) = saved.single()
            assertEquals(target.targetKey, savedTarget.targetKey)
            assertEquals(AccessAccount("own"), change.account)
            assertEquals(mapOf(CAP_COMMANDS to ACCESS_ALLOW, CAP_TERMINAL to ACCESS_ALLOW), change.capabilities)
            assertEquals(emptyList<String>(), change.programs)
            assertEquals(ValidUntil.None, change.validUntil)
        }
        // What the bridge receives: the own account, both decisions and `validUntil: null`.
        val args = setAccessArgs(target.targetKey, saved.single().second)
        assertEquals(target.targetKey, args.text("targetKey"))
        assertEquals("own", args.field("account").obj().text("kind"))
        assertEquals(ACCESS_ALLOW, args.field("capabilities").obj().text(CAP_COMMANDS))
        assertEquals(ACCESS_ALLOW, args.field("capabilities").obj().text(CAP_TERMINAL))
        assertEquals(JsonNull, args["validUntil"])
        TaskReport.note("share.setAccess", args.toString())
        // The bridge refuses a change of an unknown device, and one that names nothing, before
        // anything is stored.
        coreTest {
            Api.failure(
                "share.setAccess",
                args("targetKey" to "direct/x/y", "capabilities" to mapOf(CAP_COMMANDS to ACCESS_ALLOW)),
                "not_found",
            )
            Api.failure("share.setAccess", args("targetKey" to "direct/x/y"), "invalid")
        }
    }
}
