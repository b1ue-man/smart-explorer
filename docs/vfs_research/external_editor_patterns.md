# External-editor temp-file lifecycle — how production apps do it safely

Research for Smart Explorer's "Temp-Kopie" open mode (the temp-clone + watch +
upload-on-save path in `docs/REMOTE_EDIT.md`). Goal: harden the flow with
(1) a **unique id per edit**, (2) **never reuse** a prior local file (each open =
fresh download), (3) **delete** the local temp file + folder after it has been
saved back — safely, without losing edits.

The canonical production analog is **Cyberduck / Mountain Duck's "Edit"** feature:
download a remote file to a local temp copy, open it in the OS editor, watch it,
upload on every save. Everything below is grounded in their docs + bug tracker,
plus email-client (Outlook/Thunderbird) behavior and OS-level delete-while-open
semantics. URLs and quotes inline.

---

## 1. Cyberduck / Mountain Duck — the gold-reference edit-and-upload lifecycle

### 1.1 The happy path (what it does)

From the official Edit docs:

> "The file will be downloaded to a temporary directory and opened with the
> preferred editor."
> — https://docs.cyberduck.io/cyberduck/edit/

> "The file will be uploaded to the server **every time you choose _File → Save_**
> in the Editor application. The file is **not** changed on the server if you just
> close the document without saving it or if the content has not changed."
> — https://docs.cyberduck.io/cyberduck/edit/

So the lifecycle is:

1. **Download** the remote file to a local temp working copy.
2. **Launch** the user's preferred external editor on that copy.
3. **Watch** the copy; on each save (file modified on disk) **re-upload** it to the
   same remote path. Saves can happen *many times* in one session — the editor
   stays open and the same temp file is overwritten repeatedly.
4. The session ends when the **editor is closed** (the watched process exits).
5. **Cleanup** happens at session end (see 1.3 — and the pitfalls in §2).

Cyberduck does **not** treat "first save" as the end of the edit. The temp file
must survive for the *entire* editor lifetime because the user may save again.
This is the single most important design fact for us.

### 1.2 Temp-file *naming* — collision avoidance (directly relevant to requirement 1)

Cyberduck already solved the "two same-named files collide" problem. From the
naming-scheme issue:

> Cyberduck "saves the entire remote path in the filename, converting slashes to
> underscores" … "done to prevent name conflicts among files that have the same
> name but are in different directories, which, in the previous builds, were being
> downloaded as 'filename-N.extension'."
> — https://github.com/iterate-ch/cyberduck/issues/1101

Takeaways:

- The **old** scheme (`filename-N.extension`, bumping N on collision) is the naive
  approach and was *replaced* because N-bumping reuses/guesses names and is
  fragile. **Do not** copy the old scheme.
- Encoding the full remote path disambiguates same-named files, but it is **not a
  guaranteed-unique id** across *concurrent edits of the same remote file* or
  across sessions. For Smart Explorer's stated requirement, prefer a **unique
  per-edit directory** (random/UUID) that *contains* the original filename, so the
  editor still shows the real name in its title bar. (See §4.)

### 1.3 Where the temp lives, and when it's cleaned up

Buffered/temporary edit files live under the app cache:

> "Buffered files are saved in the folder _Temporary_ in the Cache Location."
> macOS: `~/Library/Application Support/Mountain Duck/Volumes.noindex`;
> Windows: `%LocalAppData%\Cyberduck`.
> — https://docs.cyberduck.io/mountainduck/preferences/

Cleanup on the Cyberduck (browser-edit) side is **move-to-Trash on editor close**:

> "CyberDuck moves files from a temporary folder to the Trash whenever an Editor is
> closed" … after upload, on close it "sees the file hasn't changed since the last
> upload" and then decides what to do with the temporary file. A user requests
> "an option in the Preferences to permanently delete those files, instead of
> moving them to the trash, upon successful upload to the server."
> — https://github.com/iterate-ch/cyberduck/issues/3264

For Mountain Duck (the mount product), buffered content is tied to **application
lifetime**:

> enabling buffering "allows buffering file contents in a temporary location which
> is **only deleted when quitting the application**."
> — https://docs.cyberduck.io/mountainduck/preferences/

So the two Duck products bracket the design space:

- **Cyberduck "Edit":** cleanup is **per-edit, triggered by the editor closing**
  (move temp → Trash; only if unchanged-since-last-upload). This is the model
  Smart Explorer wants.
- **Mountain Duck buffering:** cleanup is deferred to **app quit** — simpler, but
  it *leaks* during the session (see the bug in §2.3).

### 1.4 How a save is detected

Save detection is OS-native file watching. Cyberduck uses the platform's
directory/file-change APIs (FSEvents on macOS, `ReadDirectoryChangesW` on Windows)
to notice the temp file was modified, then uploads.
— https://docs.cyberduck.io/cyberduck/edit/ (upload-on-Save behavior) and general
file-watcher background (FSEvents / `ReadDirectoryChangesW`) from
https://github.com/emcrisostomo/fswatch

Smart Explorer's `RemoteEdit` / `poll_remote_edits` with a 1.5 s debounce
(`docs/REMOTE_EDIT.md`) is the same idea; the important refinement is that **a
detected modification re-uploads but must NOT trigger cleanup** — only editor-exit
does.

### 1.5 How the edited copy goes back to the remote

The upload-on-save replaces the file at the **same remote path**, and it needs
write permission on the **parent directory**, not just the file — i.e. it behaves
like delete-and-re-create rather than an in-place rewrite:

> "if I edit a file that I have write permission on, but do not have write
> permission on the file's directory, when I try to save, I get an upload failed
> message" … "This behavior suggests that Cyberduck is removing the file and
> uploading a new one, instead of editing the existing file."
> — https://github.com/iterate-ch/cyberduck/issues/8757

Relevant for us: the **remote** write should be **atomic** where the backend
supports it (upload to a temporary remote name, then rename over the target) so a
crash mid-upload doesn't truncate the user's only remote copy. (This is the same
"upload temp name + rename" pattern Cyberduck offers for watch-folders:
https://docs.duck.sh/cyberduck/edit/.) Note this is the **remote** side; it's
orthogonal to local temp hygiene but worth doing for safety.

---

## 2. Documented pitfalls (the three failure modes)

### 2.1 PREMATURE DELETE → lost edits / failed save (the central hazard)

If you delete the local temp the moment the first save uploads, the editor still
has the file open and the *next* save fails or is lost. This is a **real, repeated
Cyberduck bug**:

> "After upgrading … users encountered errors when saving files multiple times
> during an editing session. Editors reported that files had been 'removed' or were
> 'in the trash' on subsequent save attempts." … Cyberduck "appears to be deleting
> the temporary file between save operations, breaking the editor's ability to
> overwrite it."
> — https://github.com/iterate-ch/cyberduck/issues/5370 ("Cyberduck erases temp
> file while editing")

> "files can initially be updated, but then after some time (minutes) the file will
> be listed as (deleted.)" … sometimes "no notification appears, but changes
> aren't actually saved to the server." "At this point the app is unusable."
> — https://github.com/iterate-ch/cyberduck/issues/11086 ("Temporary local file
> deleted after editing in external editor")

Lesson: **deletion must be gated on the editor being truly done**, not on "a save
happened." A single save is not the end of the session.

### 2.2 CLOBBER / wrong-file (reuse hazard) — why "never reuse" matters

Thunderbird shows two clobber classes:

- **Reusing/guessing names collides.** Opening an attachment writes `Voice.PDF`,
  then `Voice-1.PDF` … up to `Voice-9999.PDF`; if a stale file with that name
  exists, opening fails:
  > "If rename to Voice-9999.PDF fails, following dialog is shown, and 'open
  > attachment by application' fails."
  > — https://bugzilla.mozilla.org/show_bug.cgi?id=673703
  Same-named temp files from different messages "can collide in temp directories,
  and stale temp files may not be properly cleaned up, potentially causing wrong
  file versions to be sent."
  — https://bugzilla.mozilla.org/show_bug.cgi?id=378046

- **Editing the *original* instead of a copy.** When the snapshot isn't taken at
  open time, the user edits the wrong file:
  > "files are not attached at *attachment time*, but *LATER* (during send)."
  > — https://bugzilla.mozilla.org/show_bug.cgi?id=378046

Lesson: **always take a fresh copy at open** into a **fresh unique location**;
never resolve to a name that might already exist or be reused. This is exactly
Smart Explorer's requirements (2) no-reuse and (1) unique-id.

### 2.3 NEVER DELETE → leak (and a security leak)

The opposite failure: defer cleanup so far it never happens.

- Mountain Duck leaks decrypted/temp content during the whole session:
  > the temporary folder "is not automatically emptied" and "The only way to empty
  > this TEMP folder is to close Mountain Duck directly." (with Cryptomator, "there
  > is still a trace of the encrypted file in my temporary folder," consuming disk).
  > — https://github.com/iterate-ch/cyberduck/issues/11761 (labeled core /
  > high-priority)
- General transfer temp files also accumulate:
  https://github.com/iterate-ch/cyberduck/issues/7778 ("Temporary files created
  during transfers are not cleaned up automatically").

Outlook has the same disease — temp attachments survive exit if the file is still
open or Outlook crashes:

> "When exiting (or when Outlook closes unexpectedly) while email attachments are
> open, the attachments remain in the Outlook Secure Temporary File folder. (Even
> if the attachments are closed.)" … cause: "The temporary files … cannot be
> deleted or removed while the attachments are open."
> — https://learn.microsoft.com/en-us/previous-versions/troubleshoot/outlook/attachments-issues-outlook

And the security consequence of leaked temp content:

> "If the attachments are deleted, opened and then closed or Outlook has been shut
> down accidentally, a copy of these attachments gets stored at the temporary
> folder location, and a person with knowledge of this location can easily gather
> confidential information."
> — https://www.remosoftware.com/info/clear-outlook-temp-folder (Outlook SecureTemp)

Lesson: leaking is both a disk problem and a **confidentiality** problem, because
the temp holds the *plaintext* of a remote file. You need a fallback sweep so a
crash/lock doesn't leak forever (§3.4) plus restrictive permissions (§5).

---

## 3. The core OS hazard: deleting a temp the editor still has open

### 3.1 Windows vs POSIX delete-while-open

This is the mechanism behind §2.1/§2.3. The two OS families behave **oppositely**:

- **POSIX** — `unlink()` on an open file succeeds; the bytes live until the last
  handle closes:
  > `unlink()` "is guaranteed to unlink the file from the file system hierarchy but
  > keep the file on disk until all open instances of the file are closed."
  > — https://cmu-sei.github.io/secure-coding-standards/sei-cert-c-coding-standard/recommendations/input-output-fio/fio08-c
- **Windows** — `remove()`/`DeleteFile` on an open file is refused (sharing
  violation) unless every handle was opened `FILE_SHARE_DELETE`:
  > "Code compiled for Microsoft Windows prevents the `remove()` call from
  > succeeding when the file is open, meaning that the file link will remain after
  > execution completes."
  > — (same FIO08-C page)
  > "Windows never allows you to really delete an open file; rather it is flagged as
  > **delete pending** and when the very last open handle to the file in the system
  > is closed, only then is it truly deleted."
  > — https://boostgsoc13.github.io/boost.afio/doc/html/afio/FAQ/deleting_open_files.html

CERT's portable rule:

> "To be strictly conforming and portable, `remove()` should _not_ be called on an
> open file."
> — FIO08-C (above)

### 3.2 Consequence for the delete decision

- On **Windows**, an attempt to delete the temp while the editor holds it **fails**
  (you get an error / it lingers) — so naive "delete after save" both breaks the
  next save *and* leaves a leak. Smart Explorer is Windows-targeted
  (`docs/REMOTE_EDIT.md`), so this is the dominant case.
- On **POSIX**, deleting while open *succeeds silently* but the editor keeps
  writing to a now-unlinked inode — a *later* save goes nowhere visible and is
  effectively **lost** to your watcher. Equally bad.

Either way: **do not delete until the editor is provably finished with the file.**

### 3.3 How do you *know* the editor is done? (the hard part)

Production strategies, weakest → strongest:

1. **Watch the editor *process* and delete on exit.** Cyberduck's model — cleanup
   "whenever an Editor is closed" (https://github.com/iterate-ch/cyberduck/issues/3264).
   Reliable **only when you launched a dedicated process you can wait on**. Breaks
   when the OS hands the file to an *already-running* instance (Word/VS Code/most
   GUI editors return immediately and the document lives in a process you didn't
   spawn) — this is precisely why Cyberduck's #5370/#11086 bugs happened.
2. **Delete when the file is no longer locked / no open handles.** On Windows,
   probe by attempting an exclusive open (or `DeleteFile` → "delete pending");
   success ⇒ no editor holds it ⇒ safe to remove. This is the most robust
   *Windows* signal that "the editor is done."
3. **Delete-on-next-launch sweep.** If you couldn't delete now (locked) or you
   crashed, sweep the temp root on the *next app start*: any leftover edit dirs
   whose files are now unlocked get removed. This is the standard
   "couldn't-delete-while-open → clean it up later" fallback (browsers/installers
   rely on the OS reboot-temp cleanup for the same reason:
   https://en.wikipedia.org/wiki/Temporary_folder).
4. **OS reboot/temp cleanup** as the last-resort backstop (only if the temp is
   under the OS temp dir).

The robust real-world answer is a **combination**: try (1)/(2) eagerly, and always
have (3) as the safety net. Never rely on (1) alone.

### 3.4 The re-save-after-delete trap, concretely

If you ever *do* delete after the first save and the user saves again:

- Windows: the second save may recreate the file (editor reopens for write) but
  your watcher already tore down → the second save **never uploads** (silent data
  loss — Cyberduck #11086's "no notification … changes aren't saved").
- POSIX: second save writes to the unlinked inode → invisible to the watcher →
  same silent loss.

Mitigation: keep the watcher alive for the **whole** editor lifetime, and only
release watcher + delete together, once the file is unlocked/the process exited.

---

## 4. "Open with" / download-then-open cleanup strategies (general)

Synthesizing the above into the patterns real apps use:

| Strategy | Who | When delete fires | Caveat |
|---|---|---|---|
| Delete-on-editor-exit | Cyberduck Edit | watched editor process exits | fails for already-running GUI apps |
| Delete-on-app-quit | Mountain Duck buffering, Outlook SecureTemp | host app closes | leaks for the whole session; leaks on crash |
| Delete-when-unlocked | robust Windows pattern | exclusive-open / delete-pending succeeds | needs a poll/retry loop |
| Delete-on-next-launch sweep | browsers, installers | next start of host app | requires durable record of temp root |
| OS temp + reboot cleanup | everything under %TEMP% | OS, on reboot | only if temp is under OS temp dir |

The reliable production recipe is **"try delete-when-unlocked / on-exit now; if it
can't, record it and sweep on next launch."** No single trigger is sufficient
alone — that's the lesson of every bug cited.

---

## 5. Security / hygiene

- The temp holds the **plaintext of a (possibly sensitive) remote file**. Leaking
  it is a confidentiality breach (Outlook SecureTemp quote, §2.3).
- Create temp files **owner-only**:
  > mkstemp "is opened using mode 0600, which means the file will be secure from
  > tampering"; IEEE 1003.1 specifies `S_IRUSR|S_IWUSR` (0600).
  > — https://owasp.org/www-community/vulnerabilities/Insecure_Temporary_File
- Don't drop loose files in a shared world-writable temp; make a **private
  subdirectory** first:
  > "avoid publically writable temporary directories if possible; if using a
  > publically writable directory, make a directory within it with read and write
  > permissions for the application only." Also "Always use absolute paths."
  > — (OWASP, above) and OpenStack:
  > https://security.openstack.org/guidelines/dg_using-temporary-files-securely.html
  > ("ensure the file is read/write by the creator only", `os.umask(0077)`;
  > "take care to cleanup our temporary files even in the face of errors").
- **Avoid predictable names** (TOCTOU): use a randomized unique dir, not a counter:
  > "Creating temporary files with predictable paths leaves them open to … TOCTOU
  > attacks." — (OpenStack, above)
- Prefer a per-app/per-user location over the bare system temp where feasible
  (Cyberduck/Outlook both use app-owned dirs under the user profile, §1.3).
- **Best-effort overwrite/secure-delete** of contents before removing is a nice-to-
  have for sensitive data; at minimum, *do* delete and don't move sensitive
  plaintext to a Recycle Bin/Trash that survives (Cyberduck's move-to-Trash default
  is convenient but means the plaintext lingers in Trash —
  https://github.com/iterate-ch/cyberduck/issues/3264 requests a hard-delete
  option for exactly this reason).

---

## 6. Recommended lifecycle for Smart Explorer ("Temp-Kopie" mode)

Satisfies: (1) unique id per edit, (2) never reuse a prior local file, (3) delete
after remote save — without losing edits.

**OPEN**
1. Generate a **fresh unique edit id** per open — e.g. a UUID/random token. Never a
   counter, never derived solely from the filename (avoids §2.2 collisions and
   §1.2's rejected `-N` scheme).
2. Create a **private per-edit directory**: `<temp-root>/<edit-id>/`, made
   **owner-only** (0700 / restrictive ACL) *before* writing anything (§5).
3. Inside it, write the working copy under the file's **real name**
   (`<temp-root>/<edit-id>/<original_name.ext>`) so the editor's title bar shows the
   correct name. Uniqueness comes from the **directory**, not a mangled filename.
4. **Always download fresh** from the backend into that file — even if a same-named
   edit dir existed before, you use a *new* id, so there is no reuse (requirement 2;
   avoids stale-copy bugs like Thunderbird #378046).
5. Record the edit (id, dir, remote path, last-uploaded mtime/hash) in a **durable
   index** so a sweep can clean it later even after a crash.

**WATCH + SAVE (entire editor lifetime)**
6. Watch the working file (FSEvents / `ReadDirectoryChangesW`; your existing
   `poll_remote_edits` + 1.5 s debounce). On each detected save where
   content/mtime changed, **upload to the remote** — ideally atomically (upload to a
   temporary remote name, rename over target; §1.5) so a failed upload can't
   truncate the user's only remote copy. Update last-uploaded mtime/hash.
7. **Allow many saves.** A save **never** triggers cleanup. The watcher stays live
   for the whole session (this is the fix for Cyberduck #5370/#11086, §2.1/§3.4).

**END + DELETE**
8. Decide "editor is done" with a **combined** signal, not just one:
   - If you launched a dedicated editor process, **wait for it to exit** (Cyberduck
     model, §3.3-1).
   - Otherwise (or additionally), **poll for unlock**: on Windows, periodically try
     an exclusive open / `DeleteFile`; success ⇒ no handle holds it ⇒ safe (§3.3-2).
9. Before deleting, **flush any final pending save** (debounce window) so the last
   edit is uploaded — don't race the watcher teardown against the last save.
10. **Delete the working file, then `rmdir` the per-edit directory** (the whole
    `<edit-id>/` folder). Hard delete (don't move plaintext to Trash/Recycle —
    §5 / #3264). On Windows this only succeeds once unlocked, which is why step 8
    gates it.
11. **Remove the entry from the durable index** only after the dir is gone.

**SAFETY NET (the leak-proofing)**
12. On **app start** (and optionally on disconnect), **sweep** the temp root: for
    each index entry / leftover `<edit-id>/` dir whose file is now unlocked, delete
    it. This recovers from crashes, OS-refused deletes, and editor-still-open cases
    (the delete-on-next-launch pattern, §4) — so you never leak forever (the
    Mountain Duck #11761 / Outlook failure mode, §2.3).
13. Because each dir is under the temp root with a unique id, the sweep is safe and
    can't clobber a *live* edit (live ones are unlocked-checked / still in the index
    as active).

Net effect: unique dir per edit ⇒ no path collisions and natural cleanup unit;
fresh download every open ⇒ no reuse; delete gated on unlock + a startup sweep ⇒
deleted after remote save, reliably, with **no lost edits** and **no permanent
leak**.

---

## 7. Source list

- Cyberduck Edit docs (download to temp, upload on every Save):
  https://docs.cyberduck.io/cyberduck/edit/ · mirror https://docs.duck.sh/cyberduck/edit/
- Temp naming = full remote path, slashes→underscores, to avoid same-name
  collisions (rejecting old `filename-N`): https://github.com/iterate-ch/cyberduck/issues/1101
- Cleanup = move-to-Trash on editor close; hard-delete requested:
  https://github.com/iterate-ch/cyberduck/issues/3264
- Cache/Temporary location + "deleted only when quitting the application":
  https://docs.cyberduck.io/mountainduck/preferences/
- PREMATURE DELETE bugs (multi-save breaks; file goes "(deleted.)"):
  https://github.com/iterate-ch/cyberduck/issues/5370 ·
  https://github.com/iterate-ch/cyberduck/issues/11086
- LEAK bugs (temp not emptied; trace of decrypted file): https://github.com/iterate-ch/cyberduck/issues/11761 ·
  https://github.com/iterate-ch/cyberduck/issues/7778
- Upload requires parent-dir write (delete-and-recreate semantics):
  https://github.com/iterate-ch/cyberduck/issues/8757
- Thunderbird name-collision / 10k temp limit: https://bugzilla.mozilla.org/show_bug.cgi?id=673703
- Thunderbird edits original instead of fresh copy: https://bugzilla.mozilla.org/show_bug.cgi?id=378046
- Outlook SecureTemp survives exit when file still open / crash:
  https://learn.microsoft.com/en-us/previous-versions/troubleshoot/outlook/attachments-issues-outlook ·
  security: https://www.remosoftware.com/info/clear-outlook-temp-folder
- Delete-while-open semantics (Windows refuses / delete-pending; POSIX unlink keeps
  inode): https://cmu-sei.github.io/secure-coding-standards/sei-cert-c-coding-standard/recommendations/input-output-fio/fio08-c ·
  https://boostgsoc13.github.io/boost.afio/doc/html/afio/FAQ/deleting_open_files.html
- Secure temp hygiene (0600, private dir, no predictable names, cleanup on error):
  https://owasp.org/www-community/vulnerabilities/Insecure_Temporary_File ·
  https://security.openstack.org/guidelines/dg_using-temporary-files-securely.html
- OS temp / reboot cleanup backstop: https://en.wikipedia.org/wiki/Temporary_folder
