package app.smartexplorer.android.ui.vaults

/**
 * Every German text of the vault screens (U31-15) and the mapping error code → text (U31-11).
 * Texts that leave the protected screens (snackbars of the list, notifications) never name a
 * file inside a vault (S2): they use only the label of the vault.
 */
object VaultTexts {
    const val MENU_TITLE = "Tresore"
    const val MENU_SUBTITLE = "Cryptomator-Tresore entsperren und verwalten"
    const val ADD = "Hinzufügen"
    const val ADD_EXISTING = "Vorhandenen Tresor eintragen"
    const val CREATE_NEW = "Neuen Tresor anlegen"
    const val LOCK_ALL = "Alle sperren"
    const val EMPTY = "Noch keine Tresore. Trage einen vorhandenen Cryptomator-Tresor ein oder lege einen neuen an."
    const val LOCKED = "Gesperrt"
    const val UNLOCKED = "Entsperrt"
    const val UNAVAILABLE = "Nicht verfügbar"
    const val UNREACHABLE = "Speicherort nicht erreichbar"
    const val UNLOCK = "Entsperren"
    const val LOCK = "Sperren"
    const val PROPERTIES = "Eigenschaften"
    const val CHECK = "Tresor prüfen"
    const val SYNC = "Synchronisieren…"
    const val REMOVE = "Austragen…"
    const val LOCAL = "Lokal"
    const val SHARE = "Share"
    const val GDRIVE = "Google Drive"

    const val LABEL = "Bezeichnung"
    const val ENTER = "Eintragen"
    const val NO_VAULT = "Kein Cryptomator-Tresor (vault.cryptomator fehlt)."
    const val HUB = "Cryptomator-Hub-Tresore werden nicht unterstützt."

    const val PASSWORD = "Passwort"
    const val REMEMBER = "Passwort auf diesem Gerät merken"
    const val REMEMBER_NEEDS_LOCK = "Nur mit eingerichteter Displaysperre möglich."
    const val UNLOCK_SAVED = "Mit gespeichertem Passwort entsperren"
    const val CANCEL = "Abbrechen"
    const val UNLOCKING = "Entsperre…"
    const val WRONG_PASSWORD = "Passwort falsch"
    const val NOT_REMEMBERED = "Passwort nicht gespeichert."
    const val SAVED_INVALID = "Das gespeicherte Passwort gilt nicht mehr, weil die Displaysperre geändert wurde. Gib das Passwort ein."
    const val PROMPT_UNLOCK = "Tresor entsperren"
    const val PROMPT_REMEMBER = "Passwort merken"
    const val SHOW_PASSWORD = "Passwort anzeigen"
    const val HIDE_PASSWORD = "Passwort verbergen"

    const val CREATE_PLACE = "Ort des Tresors"
    const val CHOOSE_PLACE = "Ort wählen"
    const val REPEAT_PASSWORD = "Passwort wiederholen"
    const val MIN_LENGTH = "Mindestens 8 Zeichen."
    const val RECOVERY_HINT = "Einen Wiederherstellungsschlüssel erzeugt Cryptomator Desktop für diesen Tresor."
    const val CREATE = "Anlegen"
    const val CREATING = "Lege Tresor an…"
    const val CREATED = "Tresor angelegt."

    const val SECTION_GENERAL = "Allgemein"
    const val SECTION_LOCK = "Sperren"
    const val SECTION_PASSWORD = "Passwort"
    const val SECTION_RECOVERY = "Wiederherstellung"
    const val SECTION_SYNC = "Synchronisation"
    const val SAVE = "Speichern"
    const val LOCK_IDLE = "Nach Leerlauf sperren"
    const val LOCK_IDLE_MINUTES = "Leerlauf"
    const val LOCK_SCREEN = "Beim Ausschalten des Bildschirms sperren"
    const val LOCK_BACKGROUND = "Im Hintergrund sperren"
    const val CHANGE_PASSWORD = "Passwort ändern"
    const val CURRENT_PASSWORD = "Aktuelles Passwort"
    const val NEW_PASSWORD = "Neues Passwort"
    const val NEW_PASSWORD_REPEAT = "Neues Passwort wiederholen"
    const val PASSWORD_KEY_SAME = "Der Schlüssel des Tresors bleibt unverändert; nur das Passwort, das ihn schützt, ändert sich."
    const val CHANGE = "Ändern"
    const val PASSWORD_CHANGED = "Passwort geändert."
    const val REMEMBER_CLEARED = "Das gespeicherte Passwort wurde entfernt; merke es beim nächsten Entsperren neu."
    const val FORGET = "Gespeichertes Passwort vergessen"
    const val FORGET_TEXT = "Das gespeicherte Passwort wird von diesem Gerät gelöscht."
    const val SYNC_SETUP = "Synchronisation einrichten…"
    const val REMOVE_BUTTON = "Tresor austragen…"
    const val REMOVE_TITLE = "Tresor austragen?"
    const val REMOVE_TEXT = "Der Tresor wird nur aus Smart Explorer entfernt. Die Dateien im Ordner bleiben unverändert."
    const val REMOVE_CONFIRM = "Austragen"

    const val UNSAVED_TITLE = "Ungespeicherte Änderungen"
    const val UNSAVED_SAVE = "Speichern und sperren"
    const val UNSAVED_KEEP = "Sperren, Änderungen verschlüsselt aufbewahren"

    const val CHECKING = "Prüfe Tresor…"
    const val NO_PROBLEMS = "Keine Probleme gefunden."
    const val BACKUP_HINT = "Die betroffenen Dateien werden vorher als Sicherung abgelegt."
    const val SYNC_MAY_EXPLAIN = "Kann ein Zwischenstand eines laufenden Syncs sein."
    const val SYNC_RUNNING = "Ein Sync läuft. Versuche es später erneut."

    const val COMPARING = "Vergleiche…"
    const val ALL_CONFLICTS_A = "Alle Konflikte: A"
    const val ALL_CONFLICTS_B = "Alle Konflikte: B"
    const val TRANSFER_NOW = "Jetzt übertragen"
    const val COMPARE = "Vergleichen"
    const val LOCKED_DURING_COMPARE = "Der Tresor wurde gesperrt. Der Vergleich wurde beendet."
    const val NO_PREVIEW = "Keine Vorschau"
    const val KEEP_BOTH_HINT = "Die andere Fassung bleibt als Konfliktkopie erhalten."
    const val MERGE = "Zusammenführen"
    const val MERGE_APPLY = "Ergebnis übernehmen"
    const val SKIP = "Überspringen"

    const val SYNC_TITLE = "Tresor-Sync einrichten"
    const val SYNC_REPLICA = "Neue Kopie, die Smart Explorer verwaltet"
    const val SYNC_FOLDER = "Neuer oder leerer Ordner"
    const val SYNC_CONFIRM = "Der Tresor-Sync überträgt verschlüsselte Dateien. Der Tresor braucht dafür nicht entsperrt zu sein."
    const val SYNC_CREATED = "Tresor-Sync angelegt."

    /** Group keys of a comparison → titles (U31-8). */
    fun groupTitle(group: String, labelA: String, labelB: String): String = when (group) {
        "conflict" -> "Konflikte"
        "changed_a" -> "Geändert auf $labelA"
        "changed_b" -> "Geändert auf $labelB"
        "new_a" -> "Neu auf $labelA"
        "new_b" -> "Neu auf $labelB"
        "deleted_a" -> "Gelöscht auf $labelA"
        "deleted_b" -> "Gelöscht auf $labelB"
        "deferred" -> "Zurückgestellt"
        "structure" -> "Struktur"
        else -> group
    }

    /** Action keys of the core (`ItemAction::key`) → button labels (U31-7, U31-9, C6.2). */
    fun actionTitle(action: String): String = when (action) {
        "keep-a" -> "Fassung A behalten"
        "keep-b" -> "Fassung B behalten"
        "keep-both" -> "Beide behalten"
        "merge" -> MERGE
        "transfer" -> TRANSFER_NOW
        "confirm-delete" -> "Löschung übernehmen"
        "restore" -> "Wiederherstellen"
        "merge-folders" -> "Ordner zusammenführen"
        "keep-folders-both" -> "Beide Ordner behalten"
        "canonical-name" -> "Kanonisch benennen"
        "remove-duplicate" -> "Doppelten Eintrag entfernen"
        "separate-folder" -> "Ordner trennen"
        "remove-node" -> "Knoten entfernen"
        "create-content" -> "Leeren Inhalt anlegen"
        "write-dirid" -> "dirid.c9r schreiben"
        "attach-orphan" -> "Unter LOST+FOUND einhängen"
        "rename-shortened" -> "Ordner umbenennen"
        "rewrite-name-file" -> "name.c9s neu schreiben"
        "remove-stage" -> "Entfernen"
        else -> action
    }

    /** Actions that delete something: the confirmation names the backup (U31-7). */
    fun removes(action: String): Boolean =
        action in setOf("remove-duplicate", "remove-node", "remove-stage", "confirm-delete", "keep-a", "keep-b")

    /** Reason keys of the `vault` event → texts (U31-10). */
    fun reasonText(reason: String?): String? = when (reason) {
        "screen_off" -> "Bildschirm aus"
        "background" -> "im Hintergrund"
        "idle" -> "Leerlauf"
        "manual" -> "von Hand"
        "process_end" -> "Prozessende"
        else -> null
    }

    fun lockedNotice(label: String, reason: String?): String =
        reasonText(reason)?.let { "$label wurde gesperrt ($it)." } ?: "$label wurde gesperrt."

    /** Error codes of C7.5 and U31-11 that need their own text; the test checks each has one. */
    val CODES = listOf(
        "vault_locked",
        "vault_use_provider",
        "vault_wrong_password",
        "vault_unsupported_format",
        "vault_hub",
        "vault_memory",
    )

    /** The text of an error [code] with the core's [message]; [label] names the vault. */
    fun errorText(code: String, message: String, label: String = ""): String = when (code) {
        "vault_locked" -> (if (label.isBlank()) "Tresor" else "Tresor $label") + " ist gesperrt. Entsperre ihn unter Mehr → Tresore."
        "vault_use_provider" -> "Dieser Tresor wird über die Dateien-App geöffnet."
        "vault_wrong_password" -> WRONG_PASSWORD
        "vault_hub" -> "Cryptomator-Hub-Tresore werden nicht unterstützt"
        // The core's texts are those of C3.1: „Tresorformat <n> nicht unterstützt“,
        // „Schlüsselableitung braucht <x> MiB, verfügbar <y> MiB“.
        "vault_unsupported_format", "vault_memory" -> message
        "vault_no_saved_password" -> "Für diesen Tresor ist kein Passwort gemerkt."
        "vault_unsaved_changes" -> UNSAVED_TITLE
        "compare_stale" -> "Der Vergleich ist veraltet. Bitte erneut vergleichen."
        "canceled" -> "Abgebrochen"
        else -> message
    }
}
