package app.smartexplorer.android.screen

/**
 * Whether typing into a focused field would replace content the accessibility service cannot see
 * ([ScreenControlService.appendText]). `ACTION_SET_TEXT` replaces the whole content, so the new
 * content is built from what the node reports: a password field reports its content masked or not at
 * all (`isPassword`), some web or custom fields report no text. Such a field takes the text only when
 * it is shown to be empty (round 11, H1): it shows its hint, or it is a password field of the
 * platform's `TextView`/`EditText` family that reports no cursor (`TextView` sets the text selection
 * only for a non-empty text, AOSP `onInitializeAccessibilityNodeInfoInternal`). A web or custom field
 * without text, hint and cursor says nothing about its content (round 12, S7 / Codex 12/1): it counts
 * as hidden. An ordinary empty `EditText` reports the text "" or its hint and is no hidden field.
 */
internal fun hidesContent(
    password: Boolean,
    text: CharSequence?,
    showingHint: Boolean,
    selectionStart: Int,
    className: CharSequence?,
): Boolean {
    val hidden = password || (text == null && !showingHint)
    if (!hidden) return false
    val platformField = className?.toString()?.let {
        it.startsWith("android.widget.") && (it.endsWith("EditText") || it.endsWith("TextView"))
    } == true
    val empty = showingHint || (password && platformField && text.isNullOrEmpty() && selectionStart < 0)
    return !empty
}
