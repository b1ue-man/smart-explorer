# W1 — Sync Root Registration Requirements (doc-grounded reference)

Scope: authoritative reference for **CfAPI sync root registration** as it applies to Smart Explorer's
`ensure_mounted` (`/home/user/smart-explorer/native/src/cfprovider.rs`, lines ~178–228), grounded in the
`cloud-filter` v0.0.6 crate source and Microsoft's `Windows.Storage.Provider` docs. This document only
describes requirements and flags issues; it does not modify code.

## Sources

Crate source (ground truth for what our calls compile to):
- `sync_root_id.rs` — `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cloud-filter-0.0.6/src/root/sync_root_id.rs`
- `sync_root_info.rs` — `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cloud-filter-0.0.6/src/root/sync_root_info.rs`

Microsoft docs:
- Register: https://learn.microsoft.com/en-us/uwp/api/windows.storage.provider.storageprovidersyncrootmanager.register
- SyncRootInfo class: https://learn.microsoft.com/en-us/uwp/api/windows.storage.provider.storageprovidersyncrootinfo
- Id: https://learn.microsoft.com/en-us/uwp/api/windows.storage.provider.storageprovidersyncrootinfo.id
- Path: https://learn.microsoft.com/en-us/uwp/api/windows.storage.provider.storageprovidersyncrootinfo.path
- IconResource: https://learn.microsoft.com/en-us/uwp/api/windows.storage.provider.storageprovidersyncrootinfo.iconresource
- Version: https://learn.microsoft.com/en-us/uwp/api/windows.storage.provider.storageprovidersyncrootinfo.version
- ProviderId: https://learn.microsoft.com/en-us/uwp/api/windows.storage.provider.storageprovidersyncrootinfo.providerid
- ProtectionMode: https://learn.microsoft.com/en-us/uwp/api/windows.storage.provider.storageprovidersyncrootinfo.protectionmode
- HydrationPolicy: https://learn.microsoft.com/en-us/uwp/api/windows.storage.provider.storageproviderhydrationpolicy
- PopulationPolicy: https://learn.microsoft.com/en-us/uwp/api/windows.storage.provider.storageproviderpopulationpolicy
- CF_SYNC_ROOT_PROVIDER_INFO (native, name/version length): https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_sync_root_provider_info

---

## 1. Required vs optional `SyncRootInfo` properties

### What the crate enforces (`register()`)

`sync_root_id.rs:159-177`. Before calling the WinRT `Register`, the crate runs a `check_field!`
macro that rejects empty values for exactly **four** fields:

```rust
// sync_root_id.rs:160-173
macro_rules! check_field {
    ($info:ident, $field:ident) => {
        if $info.$field().eq(OsStr::new("")) {
            Err(Error::new(
                ERROR_INVALID_PARAMETER.to_hresult(),
                concat!(stringify!($field), " cannot be empty"),
            ))?;
        }
    };
}
check_field!(info, display_name);
check_field!(info, icon);
check_field!(info, version);
check_field!(info, path);
```

So the crate requires non-empty: **display_name, icon, version, path**. The doc comment on
`register` (sync_root_id.rs:156-158) repeats this: *"`SyncRootInfo::display_name`,
`SyncRootInfo::icon`, `SyncRootInfo::version` and `SyncRootInfo::path` are required and cannot be empty."*

After the checks, the crate **sets `Id` itself** from the `SyncRootId` (it is NOT something we pass into
`SyncRootInfo`): `info.0.SetId(&self.0).unwrap();` then `StorageProviderSyncRootManager::Register(&info.0)`
(sync_root_id.rs:175-176). So `Id` is effectively mandatory too, but the crate guarantees it.

### What Microsoft documents as required

Microsoft's `Register` page is thin — it only documents one hard constraint in Remarks:

> "Multiple sync roots cannot be registered with the same path."
> — Register, Remarks.

`Id` is documented as the unique identity used "to manage and track the sync root throughout its
lifecycle" (Id page). Microsoft does not publish a formal required/optional table; the de-facto required
set for a working CfAPI sync root is **Id + Path** (identity + on-disk root), with
DisplayNameResource/IconResource/Version strongly expected by the Shell. The crate's choice to also force
`display_name`/`icon`/`version` non-empty is stricter than the bare WinRT contract but matches real-world
provider requirements.

### Summary table

| Property | Crate requires non-empty? | Our code sets it? | Notes |
|---|---|---|---|
| Id | Set automatically by crate from `SyncRootId` | indirectly (via builder) | sync_root_id.rs:175 |
| Path | yes | yes | `.with_path(&local_root)` |
| DisplayNameResource | yes | yes | `.with_display_name(label)` |
| IconResource | yes | yes | `.with_icon(...)` |
| Version | yes | yes | `.with_version("1.0.0")` |
| HydrationPolicy | no (defaulted) | yes | `Full` |
| PopulationPolicy | no (defaulted) | yes | `Full` |
| ProtectionMode | no (defaulted) | **no** | see §2 |
| ProviderId (GUID) | no | **no** | see §2 |
| AllowPinning, InSyncPolicy, HardlinkPolicy, RecycleBinUri, ShowSiblingsAsGroup, Context/blob, HydrationPolicyModifier | no | **no** | all optional |

---

## 2. Per-property analysis (set and not-set)

### Properties we SET

**DisplayNameResource** — `.with_display_name(label)` → `SetDisplayNameResource` (sync_root_info.rs:76-86).
Docs: *"An optional display name that maps to the existing sync root registration."* (SyncRootInfo class
table). Despite being "optional" in WinRT, the crate forces it non-empty (§1). We pass the user `label`
verbatim. **OK.** Caveat: this is the end-user-facing name shown in File Explorer's navigation pane; it is
a *display* string, not an identifier, so duplicates across connections are cosmetically confusing but not
a correctness problem.

**IconResource** — `.with_icon("<SystemRoot>\System32\shell32.dll,4")` → `SetIconResource`
(sync_root_info.rs:290-302). Docs: *"A path to an icon resource for the custom state of a file or folder.
... The path to the icon resource, for example, "SyncProvider.dll,-100", or "SyncProvider.dll,-101""*
(IconResource page). Required non-empty by the crate. See §4 for the format correctness analysis. **OK
with a fidelity caveat** (the doc examples use negative resource IDs, we use a positive index).

**HydrationPolicy = Full** — `.with_hydration_type(HydrationType::Full)` →
`SetHydrationPolicy(StorageProviderHydrationPolicy::Full)` (sync_root_info.rs:249-251, 392-401).
Docs (HydrationPolicy enum): `Full` (value 2) = *"Full hydration is performed. Ensures that the
placeholder is available locally before completing a request."* Default if unset is `Partial` (value 0) =
*"Hydration is performed at the user's request. Hydration does not continue in the background."* For a
remote-backed provider that fetches whole files on access, `Full` is a reasonable, safe choice. **OK.**

**PopulationPolicy = Full** — `.with_population_type(PopulationType::Full)` →
`SetPopulationPolicy(StorageProviderPopulationPolicy::Full)` (sync_root_info.rs:173-175, 435-442).
Docs (PopulationPolicy enum): `Full` (value 1) = *"If the placeholder files or directories are not fully
populated, the platform will request that the sync provider populate them before completing a user
request."* This is the **on-demand** policy (see §5). **OK and correct for our design.**

**Version** — `.with_version("1.0.0")` → `SetVersion` (sync_root_info.rs:189-193). Docs: *"The version
number of the sync root. ... A string value for the version number. E.g., "1.0""*. Length limit comes from
the native layer: `CF_MAX_PROVIDER_VERSION_LENGTH` = **255 characters** (CF_SYNC_ROOT_PROVIDER_INFO:
*"ProviderVersion is an end-user facing string with a maximum length of CF_MAX_PROVIDER_VERSION_LENGTH
(255 characters)."*). `"1.0.0"` is trivially within limits. **OK.**

**Path** — `.with_path(&local_root)` → `SetPath` (sync_root_info.rs:146-157). Docs: *"A storage folder
that represents the path to the root of the cloud based folder system."* Type is `IStorageFolder`. The
crate resolves it via `StorageFolder::GetFolderFromPathAsync(...).get()` and **returns an error if the path
is not an existing folder** (sync_root_info.rs:144-156 doc + impl). Our code does
`std::fs::create_dir_all(&local_root)` first (cfprovider.rs:188), so the folder exists. **OK.** Register
Remarks constraint: *"Multiple sync roots cannot be registered with the same path."* — each connection uses
a distinct `conn_root_dir(label)` (`cfsync.rs:40-42`, `sync_base().join(san(label))`), so paths are
distinct per sanitized label (collision risk noted in §3/§6).

### Properties we DO NOT SET (and whether omission is safe)

**ProviderId (GUID)** — `StorageProviderSyncRootInfo.ProviderId`. Docs: *"Gets or sets a GUID that
represents the ID of the storage provider."* Introduced in 1809 / contract v3.0. It is a *separate*
optional property from the string `Id`; it is **not** the providerId portion of the `Id` string. The crate
never exposes a setter for it, so we cannot set it through this API surface anyway. Omitting it is **safe**:
it is used for richer Shell features (grouping/branding) and is not required for `Register` to succeed or
for placeholders to work. **OK** (FIDELITY-only: no branding GUID).

**ProtectionMode** — Docs: *"The protection mode of the sync root registration."* Crate enum
`ProtectionMode::{Personal, Unknown}` maps to `StorageProviderProtectionMode::{Personal, Unknown}`
(sync_root_info.rs:341-367). When unset, the WinRT default is `Unknown` (the "can contain any type of file"
mode; the crate's own doc on the enum says `Unknown` = *"The sync root can contain any type of file."* and
`Personal` = *"should only contain personal files, not encrypted or business related files."*). Omitting it
leaves `Unknown`, which is the most permissive and correct default for a general remote (Google Drive/SFTP
may hold any file type). **OK.**

**AllowPinning** — Docs: *"Enables or disables the ability for files to be made available offline."*
Default is `false` when unset. If `false`, users cannot pin files to "Always keep on this device" from
Explorer. For a `Full` hydration provider this is acceptable but is a **fidelity gap** if we want users to
pin files offline. **RISK/FIDELITY** — see §6. Setter exists (`with_allow_pinning`, sync_root_info.rs:43-46)
but we don't call it.

**InSyncPolicy / supported_attribute** — Docs: *"Provides access to the sync policy."* Controls which
file attributes/timestamps are considered "in sync" and thus won't trip the platform into thinking a file
changed. Default empty is acceptable for a first cut. **OK** (potential later fidelity tuning).

**HardlinkPolicy** — Default `None` (hardlinks not allowed). **OK** for a remote that has no hardlink
concept.

**HydrationPolicyModifier** — Modifiers like `StreamingAllowed`, `ValidationRequired`,
`AutoDehydrationAllowed`, `AllowFullRestartHydration` (sync_root_info.rs:415-423). Default none. Omission is
**safe**; these are tuning knobs, not requirements.

**RecycleBinUri, ShowSiblingsAsGroup, Context (blob)** — all optional, default empty/false. **OK** to omit.

---

## 3. Sync root Id format — is `"SmartExplorer_<sanitized>"` valid?

### The Id structure built by `SyncRootIdBuilder`

`build()` (sync_root_id.rs:99-109) joins three U16 components with the separator `0x21` (`!`):

```
provider-name ! user-security-id ! account-name
```

Microsoft confirms this exact shape (Id page, Property Value):

> "An identifier in the form: [Storage Provider ID]![Windows SID]![Account ID]"
> Example: "OneDrive!S-1-1234!Personal".

Our call: `SyncRootIdBuilder::new(&pid).user_security_id(sid).build()` (cfprovider.rs:192), where
`pid = "SmartExplorer_<label-with-non-alphanumerics→_>"` (cfprovider.rs:168-174), the SID comes from
`SecurityId::current_user()`, and **account-name is left empty** (we never call `.account_name(...)`).

### Constraints and whether we satisfy them

- **No `!` in the provider portion.** `SyncRootIdBuilder::new` panics if the provider name contains `!`
  (sync_root_id.rs:68-71). Our `provider_id` replaces every non-ASCII-alphanumeric character with `_`,
  so `!` can never appear. **OK.**
- **Provider name ≤ 255 chars.** `new` panics if longer than `CF_MAX_PROVIDER_NAME_LENGTH`
  (sync_root_id.rs:62-67); the native doc confirms 255 (CF_SYNC_ROOT_PROVIDER_INFO: *"maximum length of
  CF_MAX_PROVIDER_NAME_LENGTH (255 characters)"*). Our prefix is `"SmartExplorer_"` (14 chars) + sanitized
  label; only a pathological >241-char label would trip it — but that is an **unhandled panic** path, not a
  graceful error (see §6).
- **Total Id ≤ 174 chars.** Id page Remarks: *"the maximum allowed length for an ID is 174 characters.
  Setting a longer ID can result in an error (ERROR_INSUFFICIENT_BUFFER)."* The Id = provider(≤?) + `!` +
  SID(~`S-1-5-21-...`, typically 40–50 chars) + `!` + empty account. With a 14-char prefix and a typical
  ~46-char SID, the budget for the sanitized label is roughly `174 - 14 - 1 - 46 - 1 ≈ 112` chars. **The
  crate's 255-char provider check is looser than the 174-char total-Id limit**, so a long label can pass
  the crate assert yet still fail `Register` with `ERROR_INSUFFICIENT_BUFFER`. **RISK** — see §6.
- **Character set.** Docs give no explicit allowed-character grammar beyond "no `!`" (the delimiter). Our
  sanitizer restricts the provider segment to `[A-Za-z0-9_]`, which is well within any reasonable
  interpretation. **OK.**
- **Empty account-name component.** The Id becomes `"SmartExplorer_X!S-1-...!"` (trailing empty third
  field). The format is still three `!`-delimited parts (`to_components` expects exactly 3,
  sync_root_id.rs:215-228), so this parses. Microsoft's example uses a non-empty Account ID ("Personal"),
  but an empty account id is structurally valid. **OK / minor FIDELITY** (account name is encouraged but
  "does not have any actual meaning", per builder doc sync_root_id.rs:90-92).

### Uniqueness / collision (the important one)

The Id's identity is `provider_id × SID`. Two different connection **labels that sanitize to the same
provider_id** collide. Because `provider_id` maps every non-alphanumeric char to `_`, the map is **not
injective**:

- `"My Drive"` and `"My_Drive"` and `"My-Drive"` → all become `SmartExplorer_My_Drive`.
- `"a/b"` and `"a b"` and `"a.b"` → all become `SmartExplorer_a_b`.

For the **same Windows user** (same SID, account empty), two such connections produce the **same SyncRootId**.
Consequences:
1. The second connection's `is_registered()` returns `true` (cfprovider.rs:193), so it **skips registration
   and silently reuses the first connection's sync root registration** — wrong display name/identity.
2. The on-disk paths also collide: `conn_root_dir` uses `san(label)` (cfsync.rs:40-42) with the same kind of
   sanitization, so both connections map to the **same `local_root`**, and `Register`'s
   *"Multiple sync roots cannot be registered with the same path"* rule is moot only because they resolve to
   one path — meaning the two logical connections fight over one folder and one registry/filter session.

This is a real **BUG/RISK** (severity depends on whether labels are user-controlled and can clash). See §6.

---

## 4. IconResource format correctness for `"shell32.dll,4"`

We register `"<SystemRoot>\System32\shell32.dll,4"` (cfprovider.rs:197-198).

- **Format `<module-path>,<index-or-id>` is correct.** The IconResource doc value reads:
  *"The path to the icon resource, for example, "SyncProvider.dll,-100", or "SyncProvider.dll,-101""*
  (IconResource page). So `path,number` is the documented shape and a fully-qualified path to a system DLL
  is fine.
- **Positive index vs negative resource ID.** The documented *examples* use **negative** numbers
  (`-100`, `-101`). By the universal Windows convention used by `ExtractIcon`/`SHDefExtractIcon` and the
  `.ico` resource model, a **negative** value `-N` is the **resource ID** `N`, while a **non-negative**
  value `N` is the **zero-based index** of the icon within the file. `shell32.dll,4` therefore means
  "the icon at index 4," which is the classic closed-folder icon (confirmed by the system-icon index
  reference; index 4 = folder). So `,4` is **syntactically valid and resolves to a real icon**.
- **Fidelity caveat:** Microsoft's own examples and most production providers reference icons by **negative
  resource ID** because indices are not stable across OS versions if Microsoft reorders the resource table,
  whereas resource IDs are. Using a positive index into a Microsoft-owned DLL is *valid today* but is a
  **FIDELITY/RISK** (a future shell32.dll could shift index 4). The comment in the code
  (cfprovider.rs:194-196) correctly explains *why* a non-empty icon is supplied (the crate's `check_field!`
  rejects empty, sync_root_id.rs:171). **Verdict: OK / FIDELITY.**

---

## 5. HydrationType::Full vs PopulationType::Full — semantics (critical for the design)

These are **two orthogonal axes**; do not conflate them.

### PopulationType / `StorageProviderPopulationPolicy` — about *placeholder enumeration* (namespace)

From the enum doc (verbatim):

> **Full (1):** "If the placeholder files or directories are not fully populated, the platform will request
> that the sync provider populate them before completing a user request."
>
> **AlwaysFull (2):** "The platform will assume that placeholder files and directories are always available
> locally."

Interpretation: **`PopulationPolicy::Full` is the ON-DEMAND mode.** A directory placeholder can be left
"not fully populated"; when the user enumerates it, the platform issues a
**`CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS`** callback and the provider supplies the child entries *at that
moment* (via `CfCreatePlaceholders` / the crate's `fetch_placeholders` handler). It does **NOT** mean
"pre-populate the entire tree at registration." `AlwaysFull` is the opposite: it tells the platform to
assume everything is already present locally and **not** to fire fetch-placeholder callbacks (so it would
suppress on-demand directory population).

> Source: PopulationPolicy enum page; corroborated by CfAPI fetch-placeholders semantics —
> `CF_PLACEHOLDER_CREATE_FLAG_DISABLE_ON_DEMAND_POPULATION` "When present, the directory is considered to
> have all children present locally and accessing it will not trigger FETCH_PLACEHOLDERS. When absent, the
> placeholder directory is considered partial and future access will trigger FETCH_PLACEHOLDERS."
> (https://github.com/MicrosoftDocs/win32/blob/docs/desktop-src/cfApi/cloud-files-enumerations.md and
> https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_callback_fetch_placeholders_flags)

**Implication for Smart Explorer's design:** A design that lazily populates a directory's children via a
`read_dir`-driven `populate_to` on the **FETCH_PLACEHOLDERS callback** is **consistent with
`PopulationPolicy::Full`** — that is exactly the mode in which the platform asks us to populate on demand.
Conversely, if the intent were to *eagerly* mirror the whole remote tree at mount time, `Full` would still
work but would not require it, and `AlwaysFull` would actively *disable* the on-demand callbacks we rely on.
So choosing `Full` (not `AlwaysFull`) is the **correct** policy for an on-demand populate-via-read_dir
design. (What still must be verified separately, in the callbacks audit, is that we actually *register and
implement* a FETCH_PLACEHOLDERS handler — `Full` only requests population; it does nothing if no handler
responds.)

### HydrationType / `StorageProviderHydrationPolicy` — about *file content* (data)

From the enum doc (verbatim):

> **Partial (0):** "Hydration is performed at the user's request. Hydration does not continue in the
> background."
> **Progressive (1):** "On demand hyrdration is performed. If hydration has not finished, it will continue
> in the background."
> **Full (2):** "Full hydration is performed. Ensures that the placeholder is available locally before
> completing a request."
> **AlwaysFull (3):** "If this is selected and a placeholder cannot be fully hydrated, the platform will
> fail with: ERROR_CLOUD_FILE_INVALID_REQUEST."

So hydration governs **fetching a file's bytes** when its content is read (the `FETCH_DATA` callback).
`Full` = "when content is needed, fetch the whole file before the read completes." This is independent of
how the *namespace* (directory listing) is populated.

**Net:** `PopulationType::Full` = on-demand placeholder/namespace population (per-directory, on
enumeration). `HydrationType::Full` = whole-file content fetch on first content access. Our pairing
(`Population=Full`, `Hydration=Full`) is the standard "on-demand listing + whole-file download on open"
configuration and is **internally consistent and correct** for the populate-via-read_dir design.

---

## 6. Concrete issues / risks in our registration code

Legend: **BUG** = will malfunction; **RISK** = can malfunction under inputs/conditions; **FIDELITY** =
works but lower-quality than docs intend; **OK** = correct.

- **[BUG/RISK] provider_id sanitization is not injective → SyncRootId + path collisions across connections.**
  `provider_id` maps every non-alphanumeric char to `_` (cfprovider.rs:168-174), and `conn_root_dir`
  similarly uses `san(label)` (cfsync.rs:40-42). Two distinct labels that differ only in non-alphanumeric
  characters (e.g. `"My Drive"` vs `"My-Drive"`, `"a/b"` vs `"a b"`) produce the **same SyncRootId** (same
  provider + same SID + empty account) **and** the same `local_root`. The second `ensure_mounted` then sees
  `is_registered() == true` (cfprovider.rs:193) and **silently reuses the first root** with the wrong
  identity/display name, while both connections also share one on-disk folder. Severity rises if labels are
  user-supplied. Citation: cfprovider.rs:168-174, 192-193; cfsync.rs:40-42; Id format
  (https://learn.microsoft.com/.../storageprovidersyncrootinfo.id); Register "Multiple sync roots cannot be
  registered with the same path" (Register Remarks).

- **[RISK] Total Id can exceed the 174-char limit even though the crate's 255-char provider check passes.**
  The crate only asserts provider-name ≤ 255 (sync_root_id.rs:62-67), but the *assembled* Id (provider +
  `!` + SID + `!` + account) must be ≤ 174 or `Register` may fail with `ERROR_INSUFFICIENT_BUFFER`. A long
  label (≳110+ chars after the `SmartExplorer_` prefix and a typical ~46-char SID) passes the assert but
  fails registration. Citation: Id Remarks (*"the maximum allowed length for an ID is 174 characters …
  ERROR_INSUFFICIENT_BUFFER"*); crate assert sync_root_id.rs:62-67.

- **[RISK] Unhandled panic path on pathological labels.** `SyncRootIdBuilder::new` (sync_root_id.rs:62-71)
  **panics** (not `Result`) if the provider name exceeds 255 chars. A label longer than ~241 chars after
  sanitization would panic inside `ensure_mounted` (cfprovider.rs:192) rather than returning the function's
  `Result<_, String>` error. Citation: sync_root_id.rs:62-67; call site cfprovider.rs:192.

- **[RISK] Failure cleanup unregisters a possibly-pre-existing sync root.** On `Session::new().connect(...)`
  failure, the code calls `sync_root_id.unregister()` (cfprovider.rs:217-223). But registration is only
  performed when `!is_registered()` (cfprovider.rs:193); if the root was **already registered by a previous
  successful run** (or by a colliding connection), a connect failure now will **unregister a root this call
  did not create**, potentially breaking the other live connection or a prior mount. The cleanup should only
  fire when *this* call performed the registration. Citation: cfprovider.rs:193, 217-223.

- **[FIDELITY] IconResource uses a positive index into a Microsoft-owned DLL.** `"shell32.dll,4"` is valid
  and resolves to the folder icon today, but Microsoft's documented examples use **negative resource IDs**
  (`SyncProvider.dll,-100`) which are version-stable, whereas a positive **index** can shift if shell32's
  resource ordering changes across OS builds. Functionally OK; lower fidelity/robustness than a `-resourceID`
  reference. Citation: IconResource page value text; system-icon index reference (index 4 = folder).

- **[FIDELITY] AllowPinning not set → users cannot mark files "Always keep on this device."** Default is
  `false`. For a `Full`-hydration cloud provider this is a missing feature, not a bug. Setter exists
  (`with_allow_pinning`, sync_root_info.rs:43-46) but is unused. Citation: AllowPinning (SyncRootInfo class
  table: *"Enables or disables the ability for files to be made available offline."*).

- **[FIDELITY] ProviderId GUID not set (and not settable via this crate).** No branding/grouping GUID is
  registered. Optional and harmless for correctness; the crate exposes no setter anyway. Citation: ProviderId
  page; absence of any `provider_id`/GUID setter in sync_root_info.rs.

- **[FIDELITY] Empty account-name component in the Id.** The third Id field is empty (we never call
  `.account_name(...)`). Structurally valid (parses as 3 components), but Microsoft's example and the
  builder doc encourage a meaningful account name. If a single Windows user ever needs two connections to
  the *same provider_id but different remote accounts*, the empty account name removes the one field
  designed to disambiguate them — compounding the collision risk above. Citation: builder doc
  sync_root_id.rs:90-92; Id example "OneDrive!S-1-1234!Personal".

- **[OK] Required fields satisfied.** display_name, icon, version, path are all set non-empty, so the
  crate's `check_field!` gate (sync_root_id.rs:170-173) passes; `Id` is set by the crate; the local_root
  folder is created before `with_path` resolves it (cfprovider.rs:188). Path uniqueness holds *per distinct
  sanitized label* (subject to the collision caveat above).

- **[OK] Hydration=Full / Population=Full pairing is correct and self-consistent** for an on-demand
  populate-via-read_dir design (see §5). `Population=Full` (not `AlwaysFull`) is specifically the policy
  that keeps FETCH_PLACEHOLDERS on-demand callbacks active; `Hydration=Full` fetches whole file content on
  access. Citation: PopulationPolicy + HydrationPolicy enum pages.

- **[OK] ProtectionMode omitted → defaults to `Unknown`** ("can contain any type of file"), the correct,
  most-permissive default for a general remote. Citation: ProtectionMode page; crate enum doc
  sync_root_info.rs:343-347.
