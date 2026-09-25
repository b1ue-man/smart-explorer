# Jetpack Compose + Material 3 — exact API reference (Compose BOM 2026.09.00)

Quelle: developer.android.com/reference/kotlin/androidx/compose/material3/* (via Context7-mirrored
snapshot `/websites/developer_android_reference_kotlin_androidx_compose_material3`),
developer.android.com/develop/ui/compose/* guides (via Context7
`/websites/developer_android_develop_ui_compose`), developer.android.com/jetpack/androidx/releases/
compose-material3 and compose-foundation release notes, developer.android.com/jetpack/compose/bom/
bom-mapping, mvnrepository.com (BOM→artifact version cross-check) · Abgerufen: 2026-09-25

**Scope note (read first).** developer.android.com's Kotlin reference pages document the *current
tip* of each library, which on 2026-09-25 already includes not-yet-released `material3 1.5.0-alpha`
API surface (the BOM pins **material3 1.4.0**, stable; see §0). Where the live docs and the pinned
1.4.0 disagree, this file says so explicitly under "1.4.0 vs. current docs" in the affected section.
When in doubt, prefer the classic/older shape described here — it compiles on 1.4.0; the alpha-only
shape does not.

---

## 0. Version mapping (Compose BOM 2026.09.00)

Per the official BOM→library table (`developer.android.com/jetpack/compose/bom/bom-mapping`,
checked 2026-09-25; same mapping for 2026.05.00 through 2026.09.00, i.e. it has not moved in months):

| Artifact | Version |
|---|---|
| `androidx.compose:compose-bom` | **2026.09.00** |
| `androidx.compose.material3:material3` | **1.4.0** (stable) |
| `androidx.compose.material3.adaptive:adaptive` / `-layout` / `-navigation` | 1.3.0 |
| `androidx.compose.foundation:foundation` / `foundation-layout` | **1.12.1** |
| `androidx.compose.ui:ui` | **1.12.1** |
| `androidx.compose.runtime:runtime` | **1.12.1** |

`androidx.compose.material3:material3-android:1.4.0` on mvnrepository.com lists a **Sept 2025**
release date; developer.android.com's own release-notes page (fetched summary) states **Sept 23,
2026** for the same version — same date as the newest `1.5.0-alpha29` entry directly above it on
that page, so that second date is very likely the summarizer copying the neighbouring alpha's date,
not a genuine re-release of 1.4.0. **Contradiction not resolved; not load-bearing** — what matters
for this file is that 1.4.0 is the version the BOM resolves today, and everything below is scoped to
that version, not to the live `1.5.0-alpha` docs.

The current alpha line (`material3 1.5.0-alpha16` → `1.5.0-alpha29`, Mar–Sept 2026) has **not**
shipped a stable release as of 2026-09-25 (confirmed via the release-notes page's own "current
alpha" marker). Do **not** pull `1.5.0-alphaNN` in manually; several of its API renames
(§"Recently changed" below) will break code written against 1.4.0.

**Gradle setup** (matches `docs/refs/android-toolchain.md` §8):
```kotlin
implementation(platform("androidx.compose:compose-bom:2026.09.00"))
implementation("androidx.compose.material3:material3")
implementation("androidx.compose.foundation:foundation")
implementation("androidx.compose.ui:ui")
implementation("androidx.compose.ui:ui-tooling-preview")
debugImplementation("androidx.compose.ui:ui-tooling")
```

### Defensive opt-in recommendation

An unnecessary `@OptIn` annotation is never a compile error in Kotlin. Several components below
(`ModalBottomSheet`, `DatePicker*`, `SegmentedButton*`, `BasicAlertDialog`, `PullToRefreshBox`) were
promoted from `@ExperimentalMaterial3Api` to stable at specific points in the `1.5.0-alpha` line
(per release notes) that **postdate** 1.4.0 — meaning they are plausibly still
`@ExperimentalMaterial3Api`-gated at the pinned 1.4.0, even though the live/current reference pages
no longer show the annotation (because those pages reflect the alpha tip). Because this file could
not get a 1.4.0-pinned source of truth for the exact annotation state of every one of these
(`androidx.tech`, which normally mirrors per-version KDoc, is no longer usable — see pitfall below),
**add `@OptIn(ExperimentalMaterial3Api::class)` at file/composable scope defensively wherever you
use any of those five component families**, rather than relying on the annotation being absent.

**Pitfall — do not use `androidx.tech` for per-version source.** It used to mirror androidx source
per released version; as of this check its per-artifact URLs (e.g.
`androidx.tech/artifacts/compose.material3/material3/1.4.0-source/...`) 301-redirect to an unrelated
gambling site. The domain has apparently expired and been squatted. Do not fetch it.

---

## 1. Scaffold

```kotlin
import androidx.compose.material3.Scaffold

@Composable
fun Scaffold(
    modifier: Modifier = Modifier,
    topBar: @Composable () -> Unit = {},
    bottomBar: @Composable () -> Unit = {},
    snackbarHost: @Composable () -> Unit = {},
    floatingActionButton: @Composable () -> Unit = {},
    floatingActionButtonPosition: FabPosition = FabPosition.End,
    containerColor: Color = MaterialTheme.colorScheme.background,
    contentColor: Color = contentColorFor(containerColor),
    contentWindowInsets: WindowInsets = ScaffoldDefaults.contentWindowInsets,
    content: @Composable (PaddingValues) -> Unit
): Unit
```
No `@OptIn` required (stable since long before 1.4.0).

- `content` receives `PaddingValues` that must be applied to the screen root (`Modifier.padding(it)`)
  to avoid overlap with `topBar`/`bottomBar`. Scaffold only pulls top/bottom window insets into that
  padding when `topBar`/`bottomBar` are absent — when present, the bar itself is expected to consume
  those insets (Material3 app bars do this by default via their own `windowInsets` param).
- `contentWindowInsets` default = `ScaffoldDefaults.contentWindowInsets` (safe-drawing-derived).

Minimal example:
```kotlin
Scaffold(
    topBar = { TopAppBar(title = { Text("Files") }) },
    floatingActionButton = { FloatingActionButton(onClick = {}) { Icon(Icons.Default.Add, null) } },
    snackbarHost = { SnackbarHost(snackbarHostState) },
) { innerPadding ->
    LazyColumn(Modifier.padding(innerPadding)) { /* ... */ }
}
```

---

## 2. TopAppBar / CenterAlignedTopAppBar / TopAppBarDefaults

```kotlin
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.TopAppBarDefaults

@Composable
fun TopAppBar(
    title: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    navigationIcon: @Composable () -> Unit = {},
    actions: @Composable RowScope.() -> Unit = {},
    expandedHeight: Dp = TopAppBarDefaults.TopAppBarExpandedHeight,
    windowInsets: WindowInsets = TopAppBarDefaults.windowInsets,
    colors: TopAppBarColors = TopAppBarDefaults.topAppBarColors(),
    scrollBehavior: TopAppBarScrollBehavior? = null,
    contentPadding: PaddingValues = TopAppBarDefaults.ContentPadding
): Unit

@Composable
fun CenterAlignedTopAppBar(
    title: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    navigationIcon: @Composable () -> Unit = {},
    actions: @Composable RowScope.() -> Unit = {},
    expandedHeight: Dp = TopAppBarDefaults.TopAppBarExpandedHeight,
    windowInsets: WindowInsets = TopAppBarDefaults.windowInsets,
    colors: TopAppBarColors = TopAppBarDefaults.topAppBarColors(),
    scrollBehavior: TopAppBarScrollBehavior? = null,
    contentPadding: PaddingValues = TopAppBarDefaults.ContentPadding
): Unit
```
No `@OptIn` required for these two classic overloads.

**1.4.0 vs. current docs.** The live reference also shows a `TopAppBar(title, subtitle, ...,
titleHorizontalAlignment, ...)` overload. Per release notes, `MediumFlexibleTopAppBar` /
`LargeFlexibleTopAppBar` / the `subtitle`-carrying "expressive" app bar shapes graduated from
experimental only in `1.5.0-alpha23` (July 2026) — **do not use the `subtitle` parameter against the
1.4.0 BOM**; use the classic `title`-only overload above.

`TopAppBarDefaults` scroll behaviors (all `@Composable`, all stable):
```kotlin
@Composable
fun TopAppBarDefaults.pinnedScrollBehavior(
    scrollableState: ScrollableState,
    state: TopAppBarState = rememberTopAppBarState(),
    canScroll: () -> Boolean = { true }
): TopAppBarScrollBehavior

@Composable
fun TopAppBarDefaults.enterAlwaysScrollBehavior(
    state: TopAppBarState = rememberTopAppBarState(),
    canScroll: () -> Boolean = { true }
): TopAppBarScrollBehavior

@Composable
fun TopAppBarDefaults.exitUntilCollapsedScrollBehavior(
    state: TopAppBarState = rememberTopAppBarState(),
    canScroll: () -> Boolean = { true },
    snapAnimationSpec: AnimationSpec<Float>? = TopAppBarDefaults.snapAnimationSpec,
    flingAnimationSpec: DecayAnimationSpec<Float>? = TopAppBarDefaults.flingAnimationSpec
): TopAppBarScrollBehavior
```
Wire a scroll behavior to `Scaffold`'s `topBar` via `Modifier.nestedScroll(scrollBehavior.nestedScrollConnection)`
on the `Scaffold`'s outer modifier.

Minimal example:
```kotlin
val scrollBehavior = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
Scaffold(
    modifier = Modifier.nestedScroll(scrollBehavior.nestedScrollConnection),
    topBar = {
        CenterAlignedTopAppBar(
            title = { Text("Smart Explorer") },
            scrollBehavior = scrollBehavior,
        )
    },
) { /* ... */ }
```

---

## 3. NavigationBar / NavigationBarItem

```kotlin
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem

@Composable
fun NavigationBar(
    modifier: Modifier = Modifier,
    containerColor: Color = NavigationBarDefaults.containerColor,
    contentColor: Color = MaterialTheme.colorScheme.contentColorFor(containerColor),
    tonalElevation: Dp = NavigationBarDefaults.Elevation,
    windowInsets: WindowInsets = NavigationBarDefaults.windowInsets,
    content: @Composable RowScope.() -> Unit
): Unit

@Composable
fun RowScope.NavigationBarItem(
    selected: Boolean,
    onClick: () -> Unit,
    icon: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    label: (@Composable () -> Unit)? = null,
    alwaysShowLabel: Boolean = true,
    colors: NavigationBarItemColors = NavigationBarItemDefaults.colors(),
    interactionSource: MutableInteractionSource? = null
): Unit
```
No `@OptIn` required. `NavigationBarItem` must be called inside `NavigationBar { ... }`'s `RowScope`.

```kotlin
NavigationBar {
    items.forEach { dest ->
        NavigationBarItem(
            selected = dest == current,
            onClick = { current = dest },
            icon = { Icon(dest.icon, contentDescription = dest.label) },
            label = { Text(dest.label) },
        )
    }
}
```

---

## 4. NavigationRail / NavigationRailItem

```kotlin
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem

@Composable
fun NavigationRail(
    modifier: Modifier = Modifier,
    containerColor: Color = NavigationRailDefaults.ContainerColor,
    contentColor: Color = contentColorFor(containerColor),
    header: (@Composable ColumnScope.() -> Unit)? = null,
    windowInsets: WindowInsets = NavigationRailDefaults.windowInsets,
    content: @Composable ColumnScope.() -> Unit
): Unit

@Composable
fun NavigationRailItem(
    selected: Boolean,
    onClick: () -> Unit,
    icon: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    label: (@Composable () -> Unit)? = null,
    alwaysShowLabel: Boolean = true,
    colors: NavigationRailItemColors = NavigationRailItemDefaults.colors(),
    interactionSource: MutableInteractionSource? = null
): Unit
```
No `@OptIn`. `header` is typically a `FloatingActionButton` or logo, shown above the items.

---

## 5. ModalNavigationDrawer / ModalDrawerSheet / NavigationDrawerItem / rememberDrawerState / DrawerValue

```kotlin
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.NavigationDrawerItem
import androidx.compose.material3.rememberDrawerState
import androidx.compose.material3.DrawerValue

@Composable
fun ModalNavigationDrawer(
    drawerContent: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    drawerState: DrawerState = rememberDrawerState(DrawerValue.Closed),
    gesturesEnabled: Boolean = true,
    scrimColor: Color = DrawerDefaults.scrimColor,
    content: @Composable () -> Unit
): Unit

@Composable
fun rememberDrawerState(
    initialValue: DrawerValue,
    confirmStateChange: (DrawerValue) -> Boolean = { true }
): DrawerState
// enum DrawerValue { Closed, Open }

@Composable
fun ModalDrawerSheet(
    drawerState: DrawerState,                 // required overload; handles back / predictive back
    modifier: Modifier = Modifier,
    drawerShape: Shape = /* rounded end shape */,
    drawerContainerColor: Color = /* surface */,
    drawerContentColor: Color = contentColorFor(drawerContainerColor),
    drawerTonalElevation: Dp = /* default */,
    windowInsets: WindowInsets = /* default */,
    content: @Composable ColumnScope.() -> Unit
): Unit
// A no-drawerState overload (ModalDrawerSheet(modifier = ..., content = ...)) also exists but
// does not handle back-press/predictive-back itself — prefer the drawerState overload above.

@Composable
fun NavigationDrawerItem(
    label: @Composable () -> Unit,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    icon: (@Composable () -> Unit)? = null,
    badge: (@Composable () -> Unit)? = null,
    shape: Shape = NavigationDrawerTokens.ActiveIndicatorShape.value,
    colors: NavigationDrawerItemColors = NavigationDrawerItemDefaults.colors(),
    interactionSource: MutableInteractionSource? = null
): Unit
```
No `@OptIn` for any of these (stable, long-standing API).

```kotlin
val drawerState = rememberDrawerState(DrawerValue.Closed)
val scope = rememberCoroutineScope()
ModalNavigationDrawer(
    drawerState = drawerState,
    drawerContent = {
        ModalDrawerSheet(drawerState) {
            NavigationDrawerItem(
                label = { Text("Home") },
                selected = true,
                onClick = { scope.launch { drawerState.close() } },
            )
        }
    },
) {
    // screen content; drawerState.open()/.close() are suspend funs
}
```

---

## 6. PermanentNavigationDrawer / PermanentDrawerSheet

```kotlin
import androidx.compose.material3.PermanentNavigationDrawer
import androidx.compose.material3.PermanentDrawerSheet

@Composable
fun PermanentNavigationDrawer(
    drawerContent: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit
): Unit

@Composable
fun PermanentDrawerSheet(
    modifier: Modifier = Modifier,
    drawerShape: Shape = RectangleShape,
    drawerContainerColor: Color = DrawerDefaults.standardContainerColor,
    drawerContentColor: Color = contentColorFor(drawerContainerColor),
    drawerTonalElevation: Dp = DrawerDefaults.PermanentDrawerElevation,
    windowInsets: WindowInsets = DrawerDefaults.windowInsets,
    content: @Composable ColumnScope.() -> Unit
): Unit
```
No `@OptIn`. Use for large-screen (tablet/desktop-class window) layouts; no drawer state — it is
always visible, so there is no open/close to manage.

---

## 7. LazyColumn / items(key=) / LazyListState

```kotlin
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.lazy.LazyListState

@Composable
fun LazyColumn(
    modifier: Modifier = Modifier,
    state: LazyListState = rememberLazyListState(),
    contentPadding: PaddingValues = PaddingValues(0.dp),
    reverseLayout: Boolean = false,
    verticalArrangement: Arrangement.Vertical =
        if (!reverseLayout) Arrangement.Top else Arrangement.Bottom,
    horizontalAlignment: Alignment.Horizontal = Alignment.Start,
    flingBehavior: FlingBehavior = ScrollableDefaults.flingBehavior(),
    userScrollEnabled: Boolean = true,
    content: LazyListScope.() -> Unit
): Unit

// LazyListScope extension functions:
fun <T> LazyListScope.items(
    items: List<T>,
    key: ((item: T) -> Any)? = null,
    contentType: (item: T) -> Any? = { null },
    itemContent: @Composable LazyItemScope.(item: T) -> Unit
): Unit

fun LazyListScope.items(
    count: Int,
    key: ((index: Int) -> Any)? = null,
    contentType: (index: Int) -> Any? = { null },
    itemContent: @Composable LazyItemScope.(index: Int) -> Unit
): Unit
```
No `@OptIn`. This is `androidx.compose.foundation`, not material3; long-stable, not touched by the
material3 1.5.0 churn described elsewhere in this file.

`LazyListState` (via `rememberLazyListState(initialFirstVisibleItemIndex = 0, initialFirstVisibleItemScrollOffset = 0)`):
- `val firstVisibleItemIndex: Int`, `val firstVisibleItemScrollOffset: Int`
- `suspend fun scrollToItem(index: Int, scrollOffset: Int = 0)`
- `suspend fun animateScrollToItem(index: Int, scrollOffset: Int = 0)`

Pitfall: always pass a **stable, unique `key`** (e.g. a file path or id) when items are the backing
list for a mutable file/folder listing — without it, Compose keys items by position and reorders /
insertions cause full-subtree recomposition and lost per-item state (e.g. selection, animation).

```kotlin
val listState = rememberLazyListState()
LazyColumn(state = listState) {
    items(entries, key = { it.path }) { entry ->
        ListItem(headlineContent = { Text(entry.name) })
    }
}
```

---

## 8. Modifier.combinedClickable

```kotlin
import androidx.compose.foundation.combinedClickable

fun Modifier.combinedClickable(
    enabled: Boolean = true,
    onClickLabel: String? = null,
    role: Role? = null,
    onLongClickLabel: String? = null,
    onLongClick: (() -> Unit)? = null,
    onDoubleClick: (() -> Unit)? = null,
    hapticFeedbackEnabled: Boolean = true,
    interactionSource: MutableInteractionSource? = null,
    onClick: () -> Unit
): Modifier
```
**No `@OptIn` required** — confirmed stable in the current `androidx.compose.foundation` guide
examples (no `@OptIn(ExperimentalFoundationApi::class)` on any of them), and `hapticFeedbackEnabled`
(historically the newest, most-likely-still-experimental parameter) is a plain stable `Boolean = true`
default in the current signature. `combinedClickable` performs the `LongPress` haptic itself when
`onLongClick` fires and `hapticFeedbackEnabled` is true — do not also call
`LocalHapticFeedback.current.performHapticFeedback(...)` for the same long-click, or you'll double-fire.

```kotlin
Modifier.combinedClickable(
    onClick = { openFile(entry) },
    onLongClick = { showContextMenuFor(entry) },
    onLongClickLabel = "Open context menu",
)
```

---

## 9. ModalBottomSheet + rememberModalBottomSheetState(skipPartiallyExpanded)

```kotlin
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.material3.ExperimentalMaterial3Api

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ModalBottomSheet(
    onDismissRequest: () -> Unit,
    modifier: Modifier = Modifier,
    sheetState: SheetState = rememberModalBottomSheetState(), // see below re: 1.4.0 vs current docs
    sheetMaxWidth: Dp = BottomSheetDefaults.SheetMaxWidth,
    sheetGesturesEnabled: Boolean = true,
    shape: Shape = BottomSheetDefaults.ExpandedShape,
    containerColor: Color = BottomSheetDefaults.ContainerColor,
    contentColor: Color = contentColorFor(containerColor),
    tonalElevation: Dp = 0.dp,
    scrimColor: Color = BottomSheetDefaults.ScrimColor,
    dragHandle: (@Composable () -> Unit)? = { BottomSheetDefaults.DragHandle() },
    contentWindowInsets: @Composable () -> WindowInsets = { BottomSheetDefaults.modalWindowInsets },
    properties: ModalBottomSheetProperties = ModalBottomSheetProperties(),
    content: @Composable ColumnScope.() -> Unit
): Unit

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun rememberModalBottomSheetState(
    skipPartiallyExpanded: Boolean = false,
    confirmValueChange: (SheetValue) -> Boolean = { true }
): SheetState
```
`ModalBottomSheet` carries `@ExperimentalMaterial3Api` **in the live reference right now** — confirmed
directly in the fetched signature block, not just inferred from release notes. Always
`@OptIn(ExperimentalMaterial3Api::class)`.

**1.4.0 vs. current docs — important.** The live reference now shows `rememberModalBottomSheetState`
as **deprecated** (strikethrough), replaced by a new unified `rememberBottomSheetState(initialValue,
enabledValues, confirmValueChange)`. Per release notes this rename happened across `1.5.0-alpha20`
→ `1.5.0-alpha22` (May–June 2026), i.e. entirely inside the not-yet-stable `1.5.0` line, which
postdates the pinned `1.4.0`. **Use `rememberModalBottomSheetState(skipPartiallyExpanded = true, ...)`
as shown above against the 1.4.0 BOM — `rememberBottomSheetState` almost certainly does not exist
yet at 1.4.0.** If the IDE fails to resolve `rememberModalBottomSheetState`, that's the signal the
resolved material3 version is newer than expected.

```kotlin
val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
var showSheet by remember { mutableStateOf(false) }
if (showSheet) {
    ModalBottomSheet(onDismissRequest = { showSheet = false }, sheetState = sheetState) {
        Text("Sheet content", Modifier.padding(16.dp))
    }
}
```

---

## 10. AlertDialog

```kotlin
import androidx.compose.material3.AlertDialog

@Composable
fun AlertDialog(
    onDismissRequest: () -> Unit,
    confirmButton: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    dismissButton: (@Composable () -> Unit)? = null,
    icon: (@Composable () -> Unit)? = null,
    title: (@Composable () -> Unit)? = null,
    text: (@Composable () -> Unit)? = null,
    shape: Shape = AlertDialogDefaults.shape,
    containerColor: Color = AlertDialogDefaults.containerColor,
    iconContentColor: Color = AlertDialogDefaults.iconContentColor,
    titleContentColor: Color = AlertDialogDefaults.titleContentColor,
    textContentColor: Color = AlertDialogDefaults.textContentColor,
    tonalElevation: Dp = AlertDialogDefaults.TonalElevation,
    properties: DialogProperties = DialogProperties()
): Unit
```
No `@OptIn` — stable.

```kotlin
AlertDialog(
    onDismissRequest = { showDialog = false },
    title = { Text("Delete file?") },
    text = { Text("This cannot be undone.") },
    confirmButton = { TextButton(onClick = { confirmDelete() }) { Text("Delete") } },
    dismissButton = { TextButton(onClick = { showDialog = false }) { Text("Cancel") } },
)
```

---

## 11. BasicAlertDialog

```kotlin
import androidx.compose.material3.BasicAlertDialog

@Composable
fun BasicAlertDialog(
    onDismissRequest: () -> Unit,
    modifier: Modifier = Modifier,
    properties: DialogProperties = DialogProperties(),
    content: @Composable () -> Unit
): Unit
```
The current live signature has **no** `@ExperimentalMaterial3Api` on it, and per release notes
`BasicAlertDialog` "graduated from Experimental" in `1.5.0-alpha25` (July 2026) — which postdates
1.4.0. **At the pinned 1.4.0, add `@OptIn(ExperimentalMaterial3Api::class)` defensively**; it is very
likely still required there even though the current docs no longer show it.

`onDismissRequest` fires on outside-click / back press only, not on a caller-provided dismiss button
inside `content` — wire your own button's `onClick` to your dismiss logic too.

---

## 12. OutlinedTextField / TextField (classic `String` overload)

```kotlin
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.TextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.input.PasswordVisualTransformation

@Composable
fun OutlinedTextField( // TextField(...) has the identical parameter list
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    readOnly: Boolean = false,
    textStyle: TextStyle = LocalTextStyle.current,
    label: (@Composable () -> Unit)? = null,
    placeholder: (@Composable () -> Unit)? = null,
    leadingIcon: (@Composable () -> Unit)? = null,
    trailingIcon: (@Composable () -> Unit)? = null,
    prefix: (@Composable () -> Unit)? = null,
    suffix: (@Composable () -> Unit)? = null,
    supportingText: (@Composable () -> Unit)? = null,
    isError: Boolean = false,
    visualTransformation: VisualTransformation = VisualTransformation.None,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default,
    keyboardActions: KeyboardActions = KeyboardActions.Default,
    singleLine: Boolean = false,
    maxLines: Int = if (singleLine) 1 else Int.MAX_VALUE,
    minLines: Int = 1,
    interactionSource: MutableInteractionSource? = null,
    shape: Shape = OutlinedTextFieldDefaults.shape, // TextFieldDefaults.shape for TextField
    colors: TextFieldColors = OutlinedTextFieldDefaults.colors() // TextFieldDefaults.colors()
): Unit

class PasswordVisualTransformation(val mask: Char = '•') : VisualTransformation
```
No `@OptIn`. `isError = true` recolors the indicator/label/supportingText to the error color scheme
automatically — you still supply the error message text yourself via `supportingText`.

**1.4.0 vs. current docs.** The live reference now also shows `OutlinedTextField(state: TextFieldState,
...)` — a newer overload built on `androidx.compose.foundation.text.input.TextFieldState`
(`inputTransformation`/`outputTransformation`/`onKeyboardAction: KeyboardActionHandler?` instead of
`keyboardActions: KeyboardActions`, and a `SecureTextField` composable for password entry using
`TextObfuscationMode` instead of `PasswordVisualTransformation`). This `TextFieldState`-based family
is real and current in `ui-text`/`foundation`, but this file targets the classic `value: String`
overload above since that's what the task's component list names; if new code is written against
`TextFieldState`, re-verify signatures separately — do not mix `keyboardActions` (classic) with
`onKeyboardAction` (new) parameter names.

`KeyboardOptions` recently renamed `autoCorrect: Boolean` → `autoCorrectEnabled: Boolean` (seen in
current `SecureTextField` example code); the old `autoCorrect` may already be deprecated-but-present
at 1.4.0's `foundation` version — prefer `autoCorrectEnabled` if the IDE offers it.

```kotlin
var password by remember { mutableStateOf("") }
var visible by remember { mutableStateOf(false) }
OutlinedTextField(
    value = password,
    onValueChange = { password = it },
    label = { Text("Password") },
    singleLine = true,
    isError = password.isNotEmpty() && password.length < 8,
    supportingText = { if (password.isNotEmpty() && password.length < 8) Text("Min. 8 characters") },
    visualTransformation = if (visible) VisualTransformation.None else PasswordVisualTransformation(),
    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Done),
    keyboardActions = KeyboardActions(onDone = { /* submit */ }),
    trailingIcon = {
        IconButton(onClick = { visible = !visible }) {
            Icon(if (visible) Icons.Filled.VisibilityOff else Icons.Filled.Visibility, null)
        }
    },
)
```

---

## 13. FilterChip / InputChip / AssistChip

```kotlin
import androidx.compose.material3.FilterChip
import androidx.compose.material3.InputChip
import androidx.compose.material3.AssistChip

@Composable
fun FilterChip(
    selected: Boolean,
    onClick: () -> Unit,
    label: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    leadingIcon: (@Composable () -> Unit)? = null,
    trailingIcon: (@Composable () -> Unit)? = null,
    shape: Shape = FilterChipDefaults.shape,
    colors: SelectableChipColors = FilterChipDefaults.filterChipColors(),
    elevation: SelectableChipElevation? = FilterChipDefaults.filterChipElevation(),
    border: BorderStroke? = FilterChipDefaults.filterChipBorder(enabled, selected),
    interactionSource: MutableInteractionSource? = null
): Unit

@Composable
fun InputChip(
    selected: Boolean,
    onClick: () -> Unit,
    label: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    leadingIcon: (@Composable () -> Unit)? = null,
    avatar: (@Composable () -> Unit)? = null,
    trailingIcon: (@Composable () -> Unit)? = null,
    shape: Shape = InputChipDefaults.shape,
    colors: SelectableChipColors = InputChipDefaults.inputChipColors(),
    elevation: SelectableChipElevation? = InputChipDefaults.inputChipElevation(),
    border: BorderStroke? = InputChipDefaults.inputChipBorder(enabled, selected),
    interactionSource: MutableInteractionSource? = null
): Unit

@Composable
fun AssistChip(
    onClick: () -> Unit,
    label: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    leadingIcon: (@Composable () -> Unit)? = null,
    trailingIcon: (@Composable () -> Unit)? = null,
    shape: Shape = AssistChipDefaults.shape,
    colors: ChipColors = AssistChipDefaults.assistChipColors(),
    elevation: ChipElevation? = AssistChipDefaults.assistChipElevation(),
    border: BorderStroke? = AssistChipDefaults.assistChipBorder(enabled),
    interactionSource: MutableInteractionSource? = null
): Unit
```
No `@OptIn` for any of the three. `AssistChip` has no `selected` state (it's an action chip, not a
toggle) — that's the distinguishing factor vs. `FilterChip`/`InputChip`.

```kotlin
FilterChip(
    selected = filterActive,
    onClick = { filterActive = !filterActive },
    label = { Text("Images only") },
    leadingIcon = if (filterActive) { { Icon(Icons.Filled.Done, null, Modifier.size(FilterChipDefaults.IconSize)) } } else null,
)
```

---

## 14. SegmentedButton / SingleChoiceSegmentedButtonRow

```kotlin
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api

@Composable
fun SingleChoiceSegmentedButtonRow(
    modifier: Modifier = Modifier,
    space: Dp = SegmentedButtonDefaults.BorderWidth,
    content: @Composable SingleChoiceSegmentedButtonRowScope.() -> Unit
): Unit

@Composable
fun SingleChoiceSegmentedButtonRowScope.SegmentedButton(
    selected: Boolean,
    onClick: () -> Unit,
    shape: Shape,                       // required — usually SegmentedButtonDefaults.itemShape(index, count)
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    colors: SegmentedButtonColors = SegmentedButtonDefaults.colors(),
    border: BorderStroke = SegmentedButtonDefaults.borderStroke(colors.borderColor(enabled, selected)),
    contentPadding: PaddingValues = SegmentedButtonDefaults.ContentPadding,
    interactionSource: MutableInteractionSource? = null,
    icon: @Composable () -> Unit = { SegmentedButtonDefaults.Icon(selected) },
    label: @Composable () -> Unit
): Unit
```
Per release notes, `SegmentedButton` "promoted to stable" in `1.5.0-alpha18` (April 2026) — postdates
1.4.0 — but the live signature capture did **not** show `@ExperimentalMaterial3Api` on it either way.
**Add `@OptIn(ExperimentalMaterial3Api::class)` defensively** at the pinned 1.4.0.

`shape` is a **required** parameter (no default) — always pass
`SegmentedButtonDefaults.itemShape(index = i, count = options.size)`.

```kotlin
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ViewModeRow(selected: Int, onSelect: (Int) -> Unit, options: List<String>) {
    SingleChoiceSegmentedButtonRow {
        options.forEachIndexed { index, label ->
            SegmentedButton(
                selected = index == selected,
                onClick = { onSelect(index) },
                shape = SegmentedButtonDefaults.itemShape(index = index, count = options.size),
                label = { Text(label) },
            )
        }
    }
}
```

---

## 15. Switch / Checkbox / RadioButton

```kotlin
import androidx.compose.material3.Switch
import androidx.compose.material3.Checkbox
import androidx.compose.material3.RadioButton

@Composable
fun Switch(
    checked: Boolean,
    onCheckedChange: ((Boolean) -> Unit)?,
    modifier: Modifier = Modifier,
    thumbContent: (@Composable () -> Unit)? = null,
    enabled: Boolean = true,
    colors: SwitchColors = SwitchDefaults.colors(),
    interactionSource: MutableInteractionSource? = null
): Unit

@Composable
fun Checkbox(
    checked: Boolean,
    onCheckedChange: ((Boolean) -> Unit)?,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    colors: CheckboxColors = CheckboxDefaults.colors(),
    interactionSource: MutableInteractionSource? = null
): Unit
// TriStateCheckbox(state: ToggleableState, onClick: (() -> Unit)?, ...) for the indeterminate case.

@Composable
fun RadioButton(
    selected: Boolean,
    onClick: (() -> Unit)?,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    colors: RadioButtonColors = RadioButtonDefaults.colors(),
    interactionSource: MutableInteractionSource? = null
): Unit
```
No `@OptIn` for any of the three. All three accept `null` for their change/click callback to render a
non-interactable (display-only) state — pass `null`, not a no-op lambda, when the row itself (not the
control) owns the click via `Modifier.toggleable`/`Modifier.selectable` on a parent `Row`
(recommended for full-row-tappable accessibility).

```kotlin
Row(
    Modifier.toggleable(value = checked, onValueChange = { checked = it }, role = Role.Checkbox),
    verticalAlignment = Alignment.CenterVertically,
) {
    Checkbox(checked = checked, onCheckedChange = null)
    Text("Include hidden files")
}
```

---

## 16. ListItem (headlineContent / supportingContent / leadingContent / trailingContent)

```kotlin
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults

@Composable
fun ListItem(
    headlineContent: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    overlineContent: (@Composable () -> Unit)? = null,
    supportingContent: (@Composable () -> Unit)? = null,
    leadingContent: (@Composable () -> Unit)? = null,
    trailingContent: (@Composable () -> Unit)? = null,
    colors: ListItemColors = ListItemDefaults.colors(),
    tonalElevation: Dp = ListItemDefaults.Elevation,
    shadowElevation: Dp = ListItemDefaults.Elevation
): Unit
```
No `@OptIn`. `ListItem` is **not clickable by itself** — wrap it in `Modifier.clickable`/
`combinedClickable` on the outer `Modifier`, or put it inside a `Card`/`Surface` with `onClick`.

**1.4.0 vs. current docs — important, high-risk.** The live reference currently shows a
*completely different* primary shape: `ListItem(selected: Boolean, onClick: () -> Unit, ...,
content: @Composable () -> Unit)` and a second `ListItem(checked: Boolean, onCheckedChange: ...)`
overload, where the headline slot is a generic trailing `content` lambda, not a named
`headlineContent` parameter. This is the "expressive list item" redesign; per release notes it
landed and graduated from experimental in `1.5.0-alpha23` (July 2026), which postdates 1.4.0.
Cross-checked against `composables.com`'s version-pinned `1.4.0-alpha18` docs and third-party
write-ups of the pre-1.5 API: **at 1.4.0, `headlineContent` is the correct, required first parameter**
— use the signature block above, not the live reference's `content`/`selected`/`checked` shape.

```kotlin
ListItem(
    headlineContent = { Text(entry.name) },
    supportingContent = { Text(entry.sizeLabel) },
    leadingContent = { Icon(entry.icon, contentDescription = null) },
    trailingContent = { Text(entry.modifiedLabel, style = MaterialTheme.typography.labelSmall) },
)
```

---

## 17. HorizontalDivider (Divider → HorizontalDivider/VerticalDivider)

```kotlin
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.VerticalDivider

@Composable
fun HorizontalDivider(
    modifier: Modifier = Modifier,
    thickness: Dp = DividerDefaults.Thickness,
    color: Color = DividerDefaults.color
): Unit

@Composable
fun VerticalDivider(
    modifier: Modifier = Modifier,
    thickness: Dp = DividerDefaults.Thickness,
    color: Color = DividerDefaults.color
): Unit
```
No `@OptIn`. **`Divider(...)` is deprecated** (shown with strikethrough in the live reference,
identical parameter list to `HorizontalDivider`) — use `HorizontalDivider`/`VerticalDivider`. This
rename predates material3 1.3 by a wide margin, so it is unambiguously in effect at 1.4.0; unlike the
other "recently changed" items in this file, there is no version-boundary risk here.

---

## 18. LinearProgressIndicator / CircularProgressIndicator (progress lambda overload)

```kotlin
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.CircularProgressIndicator

// Determinate — current, recommended (lambda avoids extra recomposition on every progress tick):
@Composable
fun LinearProgressIndicator(
    progress: () -> Float,
    modifier: Modifier = Modifier,
    color: Color = ProgressIndicatorDefaults.linearColor,
    trackColor: Color = ProgressIndicatorDefaults.linearTrackColor,
    strokeCap: StrokeCap = ProgressIndicatorDefaults.LinearStrokeCap,
    gapSize: Dp = ProgressIndicatorDefaults.LinearIndicatorTrackGapSize,
    drawStopIndicator: DrawScope.() -> Unit = { /* default stop-indicator dot */ }
): Unit

@Composable
fun CircularProgressIndicator(
    progress: () -> Float,
    modifier: Modifier = Modifier,
    color: Color = ProgressIndicatorDefaults.circularColor,
    strokeWidth: Dp = ProgressIndicatorDefaults.CircularStrokeWidth,
    trackColor: Color = ProgressIndicatorDefaults.circularDeterminateTrackColor,
    strokeCap: StrokeCap = ProgressIndicatorDefaults.CircularDeterminateStrokeCap,
    gapSize: Dp = ProgressIndicatorDefaults.CircularIndicatorTrackGapSize
): Unit

// Indeterminate:
@Composable
fun CircularProgressIndicator(
    modifier: Modifier = Modifier,
    color: Color = ProgressIndicatorDefaults.circularColor,
    strokeWidth: Dp = ProgressIndicatorDefaults.CircularStrokeWidth,
    trackColor: Color = ProgressIndicatorDefaults.circularIndeterminateTrackColor,
    strokeCap: StrokeCap = ProgressIndicatorDefaults.CircularIndeterminateStrokeCap,
    gapSize: Dp = ProgressIndicatorDefaults.CircularIndicatorTrackGapSize
): Unit
```
No `@OptIn`. The old `progress: Float` (plain value, not a lambda) overloads of both indicators are
**deprecated** (strikethrough in the live reference) — always pass a lambda: `progress = { fraction }`,
not `progress = fraction`. This is a common, easy-to-miss compile-time-only-a-warning trap: the
deprecated overload still compiles, so a stray `progress = someFloat` silently picks the old one.

```kotlin
LinearProgressIndicator(progress = { transferState.fraction }, modifier = Modifier.fillMaxWidth())
CircularProgressIndicator() // indeterminate spinner, no progress arg
```

---

## 19. Snackbar / SnackbarHostState.showSnackbar

```kotlin
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Snackbar
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarResult

@Composable
fun SnackbarHost(
    hostState: SnackbarHostState,
    modifier: Modifier = Modifier,
    snackbar: @Composable (SnackbarData) -> Unit = { Snackbar(it) }
): Unit

@Composable
fun Snackbar(
    modifier: Modifier = Modifier,
    action: (@Composable () -> Unit)? = null,
    dismissAction: (@Composable () -> Unit)? = null,
    actionOnNewLine: Boolean = false,
    shape: Shape = SnackbarDefaults.shape,
    containerColor: Color = SnackbarDefaults.color,
    contentColor: Color = SnackbarDefaults.contentColor,
    actionContentColor: Color = SnackbarDefaults.actionContentColor,
    dismissActionContentColor: Color = SnackbarDefaults.dismissActionContentColor,
    content: @Composable () -> Unit
): Unit

// class SnackbarHostState (create with `remember { SnackbarHostState() }`):
suspend fun SnackbarHostState.showSnackbar(
    message: String,
    actionLabel: String? = null,
    withDismissAction: Boolean = false,
    duration: SnackbarDuration =
        if (actionLabel == null) SnackbarDuration.Short else SnackbarDuration.Indefinite
): SnackbarResult
// enum SnackbarDuration { Indefinite, Long, Short }
// enum SnackbarResult { Dismissed, ActionPerformed }
```
No `@OptIn`. `showSnackbar` is `suspend` — call it from `rememberCoroutineScope().launch { }` (e.g. in
a button's `onClick`) or from a `ViewModel`'s coroutine scope, not directly inside composition.

```kotlin
val snackbarHostState = remember { SnackbarHostState() }
val scope = rememberCoroutineScope()
Scaffold(snackbarHost = { SnackbarHost(snackbarHostState) }) { padding ->
    Button(onClick = {
        scope.launch {
            val result = snackbarHostState.showSnackbar(
                message = "File deleted",
                actionLabel = "Undo",
                withDismissAction = true,
                duration = SnackbarDuration.Short,
            )
            if (result == SnackbarResult.ActionPerformed) undoDelete()
        }
    }) { Text("Delete") }
}
```

---

## 20. FloatingActionButton / ExtendedFloatingActionButton

```kotlin
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.ExtendedFloatingActionButton

@Composable
fun FloatingActionButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    shape: Shape = FloatingActionButtonDefaults.shape,
    containerColor: Color = FloatingActionButtonDefaults.containerColor,
    contentColor: Color = contentColorFor(containerColor),
    elevation: FloatingActionButtonElevation = FloatingActionButtonDefaults.elevation(),
    interactionSource: MutableInteractionSource? = null,
    content: @Composable () -> Unit
): Unit

@Composable
fun ExtendedFloatingActionButton(
    text: @Composable () -> Unit,
    icon: @Composable () -> Unit,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    expanded: Boolean = true,
    shape: Shape = FloatingActionButtonDefaults.extendedFabShape,
    containerColor: Color = FloatingActionButtonDefaults.containerColor,
    contentColor: Color = contentColorFor(containerColor),
    elevation: FloatingActionButtonElevation = FloatingActionButtonDefaults.elevation(),
    interactionSource: MutableInteractionSource? = null
): Unit
// A second ExtendedFloatingActionButton(content: @Composable RowScope.() -> Unit, ...) overload
// (no separate text/icon slots) also exists for fully custom row content.
```
No `@OptIn` for either. `expanded = false` on `ExtendedFloatingActionButton` animates it down to an
icon-only circular FAB (e.g. collapse on scroll) while keeping the same composable identity.

```kotlin
ExtendedFloatingActionButton(
    text = { Text("New folder") },
    icon = { Icon(Icons.Filled.CreateNewFolder, null) },
    onClick = { showCreateFolderDialog = true },
    expanded = listState.firstVisibleItemIndex == 0,
)
```

---

## 21. DropdownMenu / DropdownMenuItem (+ ExposedDropdownMenuBox / anchor type)

```kotlin
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuAnchorType

@Composable
fun DropdownMenu(
    expanded: Boolean,
    onDismissRequest: () -> Unit,
    modifier: Modifier = Modifier,
    offset: DpOffset = DpOffset(0.dp, 0.dp),
    scrollState: ScrollState = rememberScrollState(),
    properties: PopupProperties = MenuDefaults.DefaultMenuProperties,
    shape: Shape = MenuDefaults.shape,
    containerColor: Color = MenuDefaults.containerColor,
    tonalElevation: Dp = MenuDefaults.TonalElevation,
    shadowElevation: Dp = MenuDefaults.ShadowElevation,
    border: BorderStroke? = null,
    content: @Composable ColumnScope.() -> Unit
): Unit

@Composable
fun DropdownMenuItem(
    text: @Composable () -> Unit,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    leadingIcon: (@Composable () -> Unit)? = null,
    trailingIcon: (@Composable () -> Unit)? = null,
    enabled: Boolean = true,
    colors: MenuItemColors = MenuDefaults.itemColors(),
    contentPadding: PaddingValues = MenuDefaults.DropdownMenuItemContentPadding,
    interactionSource: MutableInteractionSource? = null
): Unit

@Composable
fun ExposedDropdownMenuBox(
    expanded: Boolean,
    onExpandedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable ExposedDropdownMenuBoxScope.() -> Unit
): Unit

// Inside ExposedDropdownMenuBoxScope, anchor the text field:
fun Modifier.menuAnchor(
    type: ExposedDropdownMenuAnchorType,
    enabled: Boolean = true
): Modifier
// object ExposedDropdownMenuAnchorType { val PrimaryNotEditable; val PrimaryEditable; val SecondaryEditable }
```
`DropdownMenu`, `DropdownMenuItem`, `ExposedDropdownMenuBox` and `Modifier.menuAnchor` show **no**
`@ExperimentalMaterial3Api` in the live reference — treat as stable, no `@OptIn` needed.

**Naming pitfall.** The parameter/type is `ExposedDropdownMenuAnchorType`, **not** `MenuAnchorType`.
Some older tutorials/StackOverflow answers (pre-1.3 material3) reference `MenuAnchorType`; that name
was renamed to `ExposedDropdownMenuAnchorType`, a rename that predates 1.4.0 (unlike the other
renames in this file), so it should apply as shown here.

**1.4.0 vs. current docs.** The live reference shows `ExposedDropdownMenu(...)` as an **extension
function on `ExposedDropdownMenuBoxScope`**. Per release notes this became true only in
`1.5.0-alpha26` (August 2026), postdating 1.4.0. At 1.4.0, call `ExposedDropdownMenu(...)` as a plain
top-level composable from inside the `ExposedDropdownMenuBox { }` content lambda — the call syntax
looks identical either way since it's invoked from within the scope receiver, so this is low-risk in
practice, but do not add an explicit `ExposedDropdownMenuBoxScope.` qualifier expecting an extension
function to already exist.

```kotlin
var expanded by remember { mutableStateOf(false) }
var selected by remember { mutableStateOf(options.first()) }
ExposedDropdownMenuBox(expanded = expanded, onExpandedChange = { expanded = it }) {
    TextField(
        value = selected,
        onValueChange = {},
        readOnly = true,
        modifier = Modifier.menuAnchor(ExposedDropdownMenuAnchorType.PrimaryNotEditable),
    )
    ExposedDropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
        options.forEach { option ->
            DropdownMenuItem(text = { Text(option) }, onClick = { selected = option; expanded = false })
        }
    }
}
```

---

## 22. PullToRefreshBox

```kotlin
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.material3.ExperimentalMaterial3Api

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PullToRefreshBox(
    isRefreshing: Boolean,
    onRefresh: () -> Unit,
    modifier: Modifier = Modifier,
    state: PullToRefreshState = rememberPullToRefreshState(),
    contentAlignment: Alignment = Alignment.TopStart,
    indicator: @Composable BoxScope.() -> Unit = {
        PullToRefreshDefaults.Indicator(
            modifier = Modifier.align(Alignment.TopCenter),
            isRefreshing = isRefreshing,
            state = state,
        )
    },
    enabled: Boolean = true,
    threshold: Dp = PullToRefreshDefaults.PositionalThreshold,
    content: @Composable BoxScope.() -> Unit
): Unit
```
Package is `androidx.compose.material3.pulltorefresh` — not plain `androidx.compose.material3`.
Per release notes, "PullToRefresh APIs" were "promoted to stable" in `1.5.0-alpha18` (April 2026),
postdating 1.4.0, even though the live signature capture shows no annotation.
**Add `@OptIn(ExperimentalMaterial3Api::class)` defensively at 1.4.0** — it is very likely still
required. `state` also needs `@OptIn` (`rememberPullToRefreshState()` is `@ExperimentalMaterial3Api`
too).

```kotlin
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FileListScreen(isRefreshing: Boolean, onRefresh: () -> Unit, entries: List<Entry>) {
    PullToRefreshBox(isRefreshing = isRefreshing, onRefresh = onRefresh) {
        LazyColumn { items(entries, key = { it.path }) { ListItem(headlineContent = { Text(it.name) }) } }
    }
}
```

---

## 23. DatePickerDialog + DatePicker + rememberDatePickerState

```kotlin
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.DatePicker
import androidx.compose.material3.rememberDatePickerState
import androidx.compose.material3.ExperimentalMaterial3Api

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DatePickerDialog(
    onDismissRequest: () -> Unit,
    confirmButton: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    dismissButton: (@Composable () -> Unit)? = null,
    shape: Shape = DatePickerDefaults.shape,
    tonalElevation: Dp = DatePickerDefaults.TonalElevation,
    colors: DatePickerColors = DatePickerDefaults.colors(),
    properties: DialogProperties = DialogProperties(usePlatformDefaultWidth = false),
    content: @Composable ColumnScope.() -> Unit
): Unit

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun rememberDatePickerState(
    initialSelectedDateMillis: Long? = null,
    initialDisplayedMonthMillis: Long? = initialSelectedDateMillis,
    yearRange: IntRange = DatePickerDefaults.YearRange,
    initialDisplayMode: DisplayMode = DisplayMode.Picker,
    selectableDates: SelectableDates = DatePickerDefaults.AllDates
): DatePickerState
// DatePicker(state: DatePickerState, modifier = Modifier, ...) is @ExperimentalMaterial3Api too.
```
Per release notes, `DatePicker` was "promoted to stable" in `1.5.0-alpha17` (April 2026), postdating
1.4.0, even though the live capture shows no annotation on `DatePickerDialog`/`rememberDatePickerState`
directly. **Add `@OptIn(ExperimentalMaterial3Api::class)` defensively at 1.4.0.**
`datePickerState.selectedDateMillis: Long?` is UTC-midnight epoch millis of the selected date — convert
via `java.time.Instant.ofEpochMilli(millis).atZone(ZoneOffset.UTC).toLocalDate()`, not the device's
local zone, to avoid an off-by-one-day bug.

```kotlin
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PickDateDialog(onPicked: (Long?) -> Unit, onDismiss: () -> Unit) {
    val state = rememberDatePickerState()
    DatePickerDialog(
        onDismissRequest = onDismiss,
        confirmButton = { TextButton(onClick = { onPicked(state.selectedDateMillis); onDismiss() }) { Text("OK") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    ) { DatePicker(state = state) }
}
```

---

## 24. Card / ElevatedCard / OutlinedCard

```kotlin
import androidx.compose.material3.Card
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.OutlinedCard

// Card has the same shape as ElevatedCard below, using CardDefaults.shape/cardColors()/cardElevation().
@Composable
fun ElevatedCard(
    onClick: () -> Unit,        // non-clickable overload: drop onClick, no enabled/interactionSource
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = CardDefaults.elevatedShape,
    colors: CardColors = CardDefaults.elevatedCardColors(),
    elevation: CardElevation = CardDefaults.elevatedCardElevation(),
    interactionSource: MutableInteractionSource? = null,
    content: @Composable ColumnScope.() -> Unit
): Unit

@Composable
fun OutlinedCard(
    onClick: () -> Unit,        // non-clickable overload also exists (no onClick/enabled/interactionSource)
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = CardDefaults.outlinedShape,
    colors: CardColors = CardDefaults.outlinedCardColors(),
    elevation: CardElevation = CardDefaults.outlinedCardElevation(),
    border: BorderStroke = CardDefaults.outlinedCardBorder(enabled),
    interactionSource: MutableInteractionSource? = null,
    content: @Composable ColumnScope.() -> Unit
): Unit
```
No `@OptIn`. Each of `Card`/`ElevatedCard`/`OutlinedCard` has **two overloads**: one with a leading
`onClick: () -> Unit` (clickable, adds `enabled`/`interactionSource`) and one without (static
container, no `enabled`/`interactionSource`). Pick the clickable one only when the whole card is
meant to be tappable — otherwise nested clickable children (e.g. an `IconButton` inside) will fight
the card's own ripple/semantics.

---

## 25. Badge / BadgedBox

```kotlin
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox

@Composable
fun Badge(
    modifier: Modifier = Modifier,
    containerColor: Color = BadgeDefaults.containerColor,
    contentColor: Color = contentColorFor(containerColor),
    content: (@Composable RowScope.() -> Unit)? = null
): Unit

@Composable
fun BadgedBox(
    badge: @Composable BoxScope.() -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable BoxScope.() -> Unit
): Unit
```
No `@OptIn`. `Badge` with `content = null` renders as a small dot; pass `content = { Text("8") }` for
a numeric badge. Always give the `Badge` a `Modifier.semantics { contentDescription = "..." }` —
badges have no default accessible label.

```kotlin
NavigationBarItem(
    icon = {
        BadgedBox(badge = { Badge { Text("3") } }) {
            Icon(Icons.Filled.Sync, contentDescription = "Sync")
        }
    },
    selected = false, onClick = {},
)
```

---

## 26. IconButton / Icon / Text / Surface / MaterialTheme

```kotlin
import androidx.compose.material3.IconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.Surface
import androidx.compose.material3.MaterialTheme

@Composable
fun IconButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    colors: IconButtonColors = IconButtonDefaults.iconButtonColors(),
    interactionSource: MutableInteractionSource? = null,
    shape: Shape = IconButtonDefaults.standardShape,
    content: @Composable () -> Unit
): Unit
// NB: a second, unrelated IconButton overload with a REQUIRED `shapes: IconButtonShapes` parameter
// also exists (an expressive "morphing" variant) — do not confuse it with the one above; the one
// above (no required shapes) is what you want for a plain icon button.

@Composable
fun Icon(
    imageVector: ImageVector,          // also overloads for Painter and (deprecated) Bitmap
    contentDescription: String?,
    modifier: Modifier = Modifier,
    tint: Color = LocalContentColor.current
): Unit

@Composable
fun Text(
    text: String,                      // also an AnnotatedString overload
    modifier: Modifier = Modifier,
    color: Color = Color.Unspecified,
    fontSize: TextUnit = TextUnit.Unspecified,
    fontStyle: FontStyle? = null,
    fontWeight: FontWeight? = null,
    fontFamily: FontFamily? = null,
    letterSpacing: TextUnit = TextUnit.Unspecified,
    textDecoration: TextDecoration? = null,
    textAlign: TextAlign? = null,
    lineHeight: TextUnit = TextUnit.Unspecified,
    overflow: TextOverflow = TextOverflow.Clip,
    softWrap: Boolean = true,
    maxLines: Int = Int.MAX_VALUE,
    minLines: Int = 1,
    onTextLayout: ((TextLayoutResult) -> Unit)? = null,
    style: TextStyle = LocalTextStyle.current
): Unit

@Composable
@NonRestartableComposable
fun Surface(
    modifier: Modifier = Modifier,
    shape: Shape = RectangleShape,
    color: Color = MaterialTheme.colorScheme.surface,
    contentColor: Color = contentColorFor(color),
    tonalElevation: Dp = 0.dp,
    shadowElevation: Dp = 0.dp,
    border: BorderStroke? = null,
    content: @Composable () -> Unit
): Unit
// Clickable/toggleable/selectable Surface overloads also exist (onClick / checked+onCheckedChange
// / selected+onClick leading params), each adding `enabled` + `interactionSource`.

@Composable
fun MaterialTheme(
    colorScheme: ColorScheme = MaterialTheme.colorScheme,
    shapes: Shapes = MaterialTheme.shapes,
    typography: Typography = MaterialTheme.typography,
    content: @Composable () -> Unit
): Unit
// object MaterialTheme { val colorScheme: ColorScheme @Composable get() ...
//                         val typography: Typography @Composable get() ...
//                         val shapes: Shapes @Composable get() ... }
```
No `@OptIn` for any of these six (all long-stable). `MaterialTheme` signature not independently
re-verified against a raw code block this session (only its description page was fetched) — it has
been unchanged in this exact shape since Material3's first stable release, so confidence is high, but
flagging per the task's "mark uncertainty" instruction. A sibling `MaterialExpressiveTheme(colorScheme,
motionScheme, shapes, typography, content)` also exists for opting into Material 3 Expressive motion —
not required unless you want that look.

---

## 27. Color schemes: dynamicLightColorScheme / dynamicDarkColorScheme / lightColorScheme / darkColorScheme / isSystemInDarkTheme

```kotlin
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.foundation.isSystemInDarkTheme

@RequiresApi(Build.VERSION_CODES.S) // API 31+; dynamic/"Material You" color is not available below
fun dynamicLightColorScheme(context: Context): ColorScheme
@RequiresApi(Build.VERSION_CODES.S)
fun dynamicDarkColorScheme(context: Context): ColorScheme

fun lightColorScheme(
    primary: Color = ColorLightTokens.Primary,
    onPrimary: Color = ColorLightTokens.OnPrimary,
    primaryContainer: Color = ColorLightTokens.PrimaryContainer,
    onPrimaryContainer: Color = ColorLightTokens.OnPrimaryContainer,
    /* ... secondary/tertiary/background/surface/error/outline/surfaceContainer* /
       primaryFixed* / secondaryFixed* / tertiaryFixed* — ~50 Color params total, all defaulted ... */
): ColorScheme
// darkColorScheme(...) has the identical parameter list, defaulted to ColorDarkTokens instead.

@Composable
fun isSystemInDarkTheme(): Boolean
```
`dynamicLightColorScheme`/`dynamicDarkColorScheme` are plain (non-`@Composable`) functions taking a
`Context` — call with `LocalContext.current`, typically once in your top-level theme composable,
gated behind `Build.VERSION.SDK_INT >= Build.VERSION_CODES.S`. `isSystemInDarkTheme()` is
`androidx.compose.foundation`, not material3.

```kotlin
@Composable
fun AppTheme(useDynamicColor: Boolean = true, content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val context = LocalContext.current
    val colorScheme = when {
        useDynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S ->
            if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        dark -> darkColorScheme()
        else -> lightColorScheme()
    }
    MaterialTheme(colorScheme = colorScheme, content = content)
}
```

---

## 28. Canvas + DrawScope.drawRect / drawText / TextMeasurer / rememberTextMeasurer

```kotlin
import androidx.compose.foundation.Canvas
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.TextMeasurer

@Composable
inline fun Canvas(modifier: Modifier, onDraw: DrawScope.() -> Unit): Unit
// = Spacer(modifier.drawBehind(onDraw)) under the hood

// DrawScope extension functions (androidx.compose.ui.graphics.drawscope):
fun DrawScope.drawRect(
    color: Color,
    topLeft: Offset = Offset.Zero,
    size: Size = this.size,
    alpha: Float = 1.0f,
    style: DrawStyle = Fill,
    colorFilter: ColorFilter? = null,
    blendMode: BlendMode = DefaultBlendMode
): Unit

// androidx.compose.ui.text.drawText (DrawScope extension), String overload:
fun DrawScope.drawText(
    textMeasurer: TextMeasurer,
    text: String,
    topLeft: Offset = Offset.Zero,
    style: TextStyle = TextStyle.Default,
    overflow: TextOverflow = TextOverflow.Clip,
    softWrap: Boolean = true,
    maxLines: Int = Int.MAX_VALUE,
    size: Size = Size.Unspecified,
    onTextLayout: (TextLayoutResult) -> Unit = {},
): Unit

@Composable
fun rememberTextMeasurer(cacheSize: Int = 8): TextMeasurer
```
No `@OptIn`. `rememberTextMeasurer()` must be called from composition (it's `@Composable`); the
resulting `TextMeasurer` is then captured into the `Canvas`/`drawBehind` lambda and used inside
`DrawScope.drawText(...)`. Multiple `drawText` overloads exist (`String` vs. `AnnotatedString`
vs. a pre-measured `TextLayoutResult`) — the `String` one above is not independently re-verified
against a raw signature block this session (only usage examples were fetched); parameter *names* not
100% guaranteed, but the call pattern (`textMeasurer` first, `text` second) is confirmed from
multiple examples.

```kotlin
val textMeasurer = rememberTextMeasurer()
Canvas(Modifier.fillMaxWidth().height(48.dp)) {
    drawRect(color = Color.LightGray)
    drawText(textMeasurer, "42%", topLeft = Offset(8f, 8f))
}
```

---

## 29. BackHandler (androidx.activity.compose)

```kotlin
import androidx.activity.compose.BackHandler

@Composable
fun BackHandler(enabled: Boolean = true, onBack: () -> Unit): Unit
```
No `@OptIn`. Part of `androidx.activity:activity-compose` (already a dependency per
`docs/refs/android-toolchain.md` §1/§8 — `androidx.activity:activity-compose:1.13.0`). Only the
**innermost enabled** `BackHandler` in the composition consumes a given back event when several are
present. For apps targeting Android 16 (this project's plan), predictive-back gesture progress needs
`PredictiveBackHandler { progress: Flow<BackEventCompat> -> ... }` instead if a custom in-flight
animation is wanted (see `docs/refs/android-platform.md` §on system bars/back handling) — `BackHandler`
alone still works for simple intercept-and-act back handling.

```kotlin
BackHandler(enabled = selectionActive) { clearSelection() }
```

---

## 30. rememberSaveable

```kotlin
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.Saver
import androidx.compose.runtime.saveable.listSaver

@Composable
fun <T : Any> rememberSaveable(
    vararg inputs: Any?,
    saver: Saver<T, out Any> = autoSaver(),
    key: String? = null,
    init: () -> T
): T
```
No `@OptIn`. Survives configuration changes and process death (backed by `Bundle`/`SavedStateHandle`),
unlike plain `remember`. Built-in `Saver`s exist for primitives/`Parcelable`/enum-ish types; for a
custom data class, provide an explicit `saver = listSaver(save = {...}, restore = {...})` — passing a
non-`Bundle`-compatible custom object with no `Saver` throws at save time (typically surfaced as an
`IllegalStateException` from state restoration, not a compile error).

```kotlin
var query by rememberSaveable { mutableStateOf("") }
```

---

## 31. LaunchedEffect / DisposableEffect / rememberCoroutineScope

```kotlin
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.rememberCoroutineScope

@Composable
fun LaunchedEffect(key1: Any?, block: suspend CoroutineScope.() -> Unit): Unit
// overloads for key1+key2, key1+key2+key3, and vararg keys also exist;
// LaunchedEffect(Unit, ...) / LaunchedEffect(true, ...) run the block exactly once per composition entry.

@Composable
fun DisposableEffect(key1: Any?, effect: DisposableEffectScope.() -> DisposableEffectResult): Unit
// effect lambda MUST end with `onDispose { ... }` — that's what DisposableEffectScope provides.

@Composable
fun rememberCoroutineScope(getContext: @DisallowComposableCalls () -> CoroutineContext = { EmptyCoroutineContext }): CoroutineScope
```
No `@OptIn`. `LaunchedEffect`/`DisposableEffect` restart their block whenever any key changes
(reference/structural-equality compared) — a key of `Unit`/`true` means "run once and never restart
for the life of this call site." `rememberCoroutineScope()` gives a scope for launching coroutines
**from event callbacks** (e.g. `onClick`), not from inside the composable body itself — for the latter,
use `LaunchedEffect`.

```kotlin
DisposableEffect(lifecycleOwner) {
    val observer = LifecycleEventObserver { _, event -> /* ... */ }
    lifecycleOwner.lifecycle.addObserver(observer)
    onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
}
```

---

## 32. collectAsStateWithLifecycle

```kotlin
import androidx.lifecycle.compose.collectAsStateWithLifecycle
```
Gradle: `implementation("androidx.lifecycle:lifecycle-runtime-compose:2.10.0")` — separate from
`lifecycle-viewmodel-compose`; `docs/refs/android-toolchain.md` §1 lists
`androidx.lifecycle:lifecycle-viewmodel-compose:2.11.0` as the pinned version for the sibling
artifact, so use the matching `2.11.0` (or whatever `lifecycle-runtime-compose` version the BOM/
version catalog resolves) rather than the `2.10.0` shown in the fetched guide snippet, to keep both
`lifecycle-*` artifacts on one release line.

```kotlin
@Composable
fun StateFlow<T>.collectAsStateWithLifecycle(
    lifecycleOwner: LifecycleOwner = LocalLifecycleOwner.current,
    minActiveState: Lifecycle.State = Lifecycle.State.STARTED,
    context: CoroutineContext = EmptyCoroutineContext
): State<T>
```
No `@OptIn`. Prefer this over plain `collectAsState()` in any screen composable — it stops collecting
(and the upstream flow's `WhileSubscribed` producer stops) when the lifecycle drops below `STARTED`
(e.g. app backgrounded), avoiding wasted work/battery that plain `collectAsState()` does not avoid.

```kotlin
val entries by viewModel.entries.collectAsStateWithLifecycle()
```

---

## 33. viewModel() + viewModelFactory { initializer { } }

```kotlin
import androidx.lifecycle.viewmodel.compose.viewModel
```
Gradle: `androidx.lifecycle:lifecycle-viewmodel-compose:2.11.0` (per toolchain doc).

```kotlin
@Composable
fun <VM : ViewModel> viewModel(
    modelClass: Class<VM> /* reified in the inline overload used below */,
    viewModelStoreOwner: ViewModelStoreOwner = checkNotNull(LocalViewModelStoreOwner.current),
    key: String? = null,
    factory: ViewModelProvider.Factory? = null,
    extras: CreationExtras = /* owner's default extras */
): VM
// Typical call site uses the reified inline convenience: viewModel<MyViewModel>() or just
// `viewModel()` with the type inferred from the property/parameter it's assigned to.
```

```kotlin
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.lifecycle.createSavedStateHandle

val Factory = viewModelFactory {
    initializer {
        val savedStateHandle = createSavedStateHandle() // extension on CreationExtras, in scope here
        MyViewModel(savedStateHandle, myRepository)
    }
}
```
`viewModelFactory`/`initializer` live in `androidx.lifecycle:lifecycle-viewmodel` (not `-compose`),
pulled in transitively by `lifecycle-viewmodel-compose`; no extra Gradle line needed. **Not
independently re-verified via a fetched primary-source code block this session** — this matches a
long-stable (`androidx.lifecycle` 2.5+) shape from training knowledge; re-confirm the exact
`initializer { }` block's implicit receiver (`CreationExtras`) against the resolved 2.11.0 jar/IDE
autocomplete before relying on it, per the task's "mark uncertainty" instruction.

```kotlin
@Composable
fun FileListScreen(viewModel: FileListViewModel = viewModel(factory = FileListViewModel.Factory)) { /* ... */ }
```

---

## 34. LocalContext

```kotlin
import androidx.compose.ui.platform.LocalContext

val LocalContext: ProvidableCompositionLocal<Context>
// usage: val context = LocalContext.current
```
No `@OptIn`. `androidx.compose.ui:ui` (part of the BOM). Avoid storing the `Context` itself in a
`remember { }` beyond the composition it was read in if it might be an `Activity` context and the
value could outlive a configuration change — read `.current` fresh where needed, or store only what
you derive from it.

---

## 35. LocalClipboard (current) vs. LocalClipboardManager (deprecated)

```kotlin
import androidx.compose.ui.platform.LocalClipboard   // current
import androidx.compose.ui.platform.LocalClipboardManager // deprecated
```
**`LocalClipboardManager` is deprecated; `LocalClipboard` is current.** `LocalClipboardManager.current`
gives a synchronous `ClipboardManager` (`setText(AnnotatedString)`, `getText(): AnnotatedString?`,
`setClip(ClipEntry)`, deprecated as a type too). `LocalClipboard.current` gives a `Clipboard` whose
operations are `suspend` (non-blocking — clipboard access can involve a cross-process call on some
Android versions):

```kotlin
val Clipboard.current: Clipboard // via LocalClipboard
interface Clipboard {
    suspend fun setClipEntry(clipEntry: ClipEntry?)
    suspend fun getClipEntry(): ClipEntry?
    // plus nativeClipboard access for platform interop
}
```
Not independently re-verified against a raw fetched signature block this session (inferred from a
consistent web-search summary plus the parallel `composables.com` "LocalClipboardManager (deprecated)
/ LocalClipboard (current)" listing) — cross-check exact `Clipboard` interface member names against
IDE autocomplete before use; the `suspend fun setClipEntry(ClipEntry?)` shape is the best-supported
detail, direction (`LocalClipboard` is the one to use for new code) is solid.

```kotlin
val clipboard = LocalClipboard.current
val scope = rememberCoroutineScope()
Button(onClick = { scope.launch { clipboard.setClipEntry(ClipEntry(ClipData.newPlainText("path", filePath))) } }) {
    Text("Copy path")
}
```

---

## 36. Modifier.horizontalScroll / rememberScrollState

```kotlin
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState

fun Modifier.horizontalScroll(
    state: ScrollState,
    enabled: Boolean = true,
    flingBehavior: FlingBehavior? = null,
    reverseScrolling: Boolean = false
): Modifier

@Composable
fun rememberScrollState(initial: Int = 0): ScrollState
```
No `@OptIn`. Unlike `LazyRow`/`LazyColumn`, this composes **all** content eagerly (no windowing) — only
use it for content that's cheap/bounded to lay out in full (e.g. a row of a handful of filter chips),
not for a potentially-large file list.

```kotlin
Row(Modifier.horizontalScroll(rememberScrollState())) { chips.forEach { FilterChip(...) } }
```

---

## 37. WindowInsets helpers: safeDrawingPadding / systemBarsPadding / imePadding

```kotlin
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.windowInsetsPadding

fun Modifier.safeDrawingPadding(): Modifier    // WindowInsets.safeDrawing as padding
fun Modifier.systemBarsPadding(): Modifier     // WindowInsets.systemBars as padding
fun Modifier.imePadding(): Modifier            // WindowInsets.ime as padding (animates with the keyboard)
fun Modifier.navigationBarsPadding(): Modifier // WindowInsets.navigationBars as padding
fun Modifier.windowInsetsPadding(insets: WindowInsets): Modifier // generic: any WindowInsets as padding
```
No `@OptIn` (these live-padding modifiers are stable; some lower-level `WindowInsets` APIs are gated
by `ExperimentalLayoutApi` — none of the five above are, per the fetched guide examples, which call
them with no `@OptIn`). Must be paired with `enableEdgeToEdge()` (§38) in `onCreate` — without it,
the system already insets the window itself and these modifiers have nothing meaningful to add
(content never draws under the bars in the first place).

Per `docs/refs/android-platform.md` §"Compose setup": content is **not** auto-padded away from system
bars/cutouts once edge-to-edge is enabled — applying one of these modifiers (or an app-bar's own
`windowInsets` handling) is mandatory, not optional polish.

```kotlin
Scaffold(
    modifier = Modifier.imePadding(), // keep FAB/input above the keyboard
    topBar = { TopAppBar(title = { Text("Files") }) }, // TopAppBar's own windowInsets already handles the status bar
) { padding -> /* content */ }
```

---

## 38. enableEdgeToEdge() (androidx.activity) + ComponentActivity.setContent

```kotlin
import androidx.activity.enableEdgeToEdge
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent

fun ComponentActivity.enableEdgeToEdge(
    statusBarStyle: SystemBarStyle = SystemBarStyle.auto(Color.TRANSPARENT, Color.TRANSPARENT),
    navigationBarStyle: SystemBarStyle = SystemBarStyle.auto(/* light/dark-aware scrim */, /* ... */)
): Unit

fun ComponentActivity.setContent(
    parent: CompositionContext? = null,
    content: @Composable () -> Unit
): Unit
```
No `@OptIn`. Both from `androidx.activity:activity-compose:1.13.0` (per toolchain doc). Call
`enableEdgeToEdge()` in `onCreate()` **before** `setContent { }`. Per
`docs/refs/android-platform.md`, this makes system bars transparent by default with a translucent
scrim kept behind 3-button nav for contrast, and the system bar icon color adapts to the app's
light/dark theme automatically — you generally do not need to set icon colors manually.

```kotlin
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            AppTheme {
                Surface(Modifier.fillMaxSize()) { AppRoot() }
            }
        }
    }
}
```

---

## Recently changed in material3 1.3 → 1.4 → 1.5 (deprecated → replacement)

| Old | New | Status at BOM 2026.09.00 (material3 1.4.0) |
|---|---|---|
| `Divider(...)` | `HorizontalDivider(...)` / `VerticalDivider(...)` | Old deprecated well before 1.3; **use the new names**, unambiguous. |
| `progress: Float` overload of `LinearProgressIndicator`/`CircularProgressIndicator` | `progress: () -> Float` lambda overload | Old deprecated (strikethrough in live docs); **use the lambda overload** — this was already true at 1.3.0/1.4.0, not a 1.5-only change. |
| `MenuAnchorType` (pre-1.3 naming) | `ExposedDropdownMenuAnchorType` | Rename predates 1.4.0; **use `ExposedDropdownMenuAnchorType`**. |
| `LocalClipboardManager` / `ClipboardManager` (sync) | `LocalClipboard` / `Clipboard` (suspend) | Old is deprecated now; exact version of the deprecation relative to 1.4.0 not pinned down this session — **prefer `LocalClipboard`** for new code regardless. |
| `rememberModalBottomSheetState(skipPartiallyExpanded, confirmValueChange)` | `rememberBottomSheetState(initialValue, enabledValues, confirmValueChange)` | New API is **1.5.0-alpha20+ only** (postdates 1.4.0) — **keep using `rememberModalBottomSheetState` at this BOM.** |
| `ExposedDropdownMenu(...)` as a top-level composable | `ExposedDropdownMenuBoxScope.ExposedDropdownMenu(...)` extension function | Extension-function form is **1.5.0-alpha26+ only** — at 1.4.0 it is (effectively) a top-level composable called from inside the box scope; call syntax is unaffected either way. |
| `ListItem(headlineContent = ..., ...)` | `ListItem(selected/checked, onClick/onCheckedChange, ..., content = ...)` "expressive" redesign | Redesign is **1.5.0-alpha23+ only** — **`headlineContent` is correct at 1.4.0.** |
| `TopAppBar(title, ...)` (no subtitle) | `TopAppBar(title, subtitle, ..., titleHorizontalAlignment, ...)` flexible/expressive app bars | `subtitle` overload graduated **1.5.0-alpha23+ only** — **use the no-subtitle overload at 1.4.0.** |
| `ModalBottomSheet`, `DatePicker*`, `SegmentedButton*`, `BasicAlertDialog`, `PullToRefreshBox` gated by `@ExperimentalMaterial3Api` | same APIs, stable (no opt-in) | Per release notes these graduate across `1.5.0-alpha17` → `alpha25` (Apr–Jul 2026) — **all postdate 1.4.0; keep `@OptIn(ExperimentalMaterial3Api::class)` on all five at this BOM** even though current docs no longer show the annotation. |

---

## Open items / unresolved for the implementer

- Exact `androidx.lifecycle:lifecycle-runtime-compose` version to pair with
  `lifecycle-viewmodel-compose:2.11.0` was not independently checked this session (§32) — resolve via
  the version catalog / BOM, not the `2.10.0` figure quoted in the fetched guide snippet.
- `viewModelFactory { initializer { } }` (§33) and the `Clipboard`/`LocalClipboard` exact member
  signatures (§35) rely on strong prior knowledge plus indirect (search-summary) confirmation, not a
  directly fetched raw signature block — worth a quick IDE-autocomplete cross-check before first use.
  `Canvas`/`drawText`'s exact parameter names (§28) are similarly only indirectly confirmed.
  `MaterialTheme`'s composable signature (§26) is high-confidence but was not seen as a raw fetched
  code block either.
  `KeyboardOptions`'s `autoCorrectEnabled` vs. `autoCorrect` naming at exactly 1.4.0 (§12) is inferred
  from one example snippet, not a fetched class signature.
- Whether `ModalBottomSheet`/`DatePicker*`/`SegmentedButton*`/`BasicAlertDialog`/`PullToRefreshBox`
  are *actually* still `@ExperimentalMaterial3Api`-gated at exactly 1.4.0 could not be pinned down
  from a 1.4.0-specific primary source this session (the only per-version source found,
  `androidx.tech`, now redirects to an unrelated site — do not use it). The defensive recommendation
  (§0, always opt in) sidesteps needing to resolve this exactly, since the annotation is harmless to
  add even when no longer required.
- material3 1.4.0's exact stabilization date is contradictory across sources (§0) and was left
  unresolved as non-load-bearing.
