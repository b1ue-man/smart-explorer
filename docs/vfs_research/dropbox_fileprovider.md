# Virtual Filesystems at Scale: Dropbox & Apple File Provider — Engineering Lessons

Research for **Smart Explorer** — choosing between **(A)** an on-demand placeholder
virtual filesystem (present remote SFTP/FTP/WebDAV/Drive files as if they were real local
files) vs **(B)** download-a-real-copy-and-save-back.

This document collects hard-won engineering experience from teams that *built* virtual
filesystems at scale — primarily **Dropbox (Project Infinite / Smart Sync)** and **Apple's
File Provider** — plus corroborating evidence from sync vendors (Nextcloud/ownCloud, rclone,
userfilesystem.com) and Apple's own developer forums and tech notes.

**Bottom line up front:** A placeholder virtual filesystem is a *major, ongoing*
engineering commitment that the largest, best-funded sync vendors found brutally hard and
were eventually forced to re-platform (kernel → user space). The complexity is justified
*only* for an always-running sync product that must make the entire backend look native to
every app on the machine. For a focused file explorer that opens/edits specific files,
option (B) (download-real-copy, save-back) avoids the entire class of failure modes
documented below.

---

## 1. Why a virtual FS is a major, ongoing engineering commitment (kernel → user space)

### 1.1 Dropbox went *into the kernel* — and explained why FUSE wasn't enough

Dropbox's first-generation Smart Sync (announced as **"Project Infinite"**, 2016) shipped a
**custom kernel extension** on macOS. Their own engineering blog explains the reasoning:

> "Since FUSE filesystems are implemented in large part in user space, any file operation
> usually requires an extra user-kernel mode switch"
> — Dropbox, *Going deeper with Project Infinite*
> https://dropbox.tech/infrastructure/going-deeper-with-project-infinite

> "After exploring the option of using FUSE, we realized that there are many benefits to
> writing our own custom kernel extension: we are able to achieve minimal performance
> overhead"
> — same source

> "FUSE is an incredible technology, but as we gained a deeper understanding it became clear
> that it didn't fully satisfy the two major constraints for our projects—world-class
> performance and rock-solid security."
> — Damien DeVille, Dropbox (quoted at https://mjtsai.com/blog/2016/05/26/dropboxs-upcoming-kernel-extension/)

> "With Dropbox Infinite, we're going deeper: into the kernel—the core of the operating
> system."
> — Damien DeVille, Dropbox (same)

To intercept file operations *before* other apps act on them, the extension hooks the
kernel's authorization layer:

> "By listening to actions on the KAUTH_SCOPE_VNODE scope, we can detect and deny actions
> that happen in the Dropbox folder"
> — https://dropbox.tech/infrastructure/going-deeper-with-project-infinite

**Takeaway:** The team that *most* wanted a clean placeholder FS concluded that the
user-space (FUSE) version was too slow because *"any file operation usually requires an
extra user-kernel mode switch"* — so they took on the enormous liability of shipping kernel
code. That liability is the heart of the cautionary tale.

### 1.2 The cost of a kernel extension — and why everyone retreated to user space

Putting a filesystem in the kernel means any bug is catastrophic. Dropbox acknowledged this:

> "Because the kernel connects applications to the physical memory, CPU, and external
> devices, any bug introduced to the kernel can adversely affect the whole machine"
> — https://dropbox.tech/infrastructure/going-deeper-with-project-infinite

Apple subsequently **deprecated kernel extensions** and pushed everyone to user-space
*System Extensions* / *File Provider*. The retrospective framing:

> "Hacking the macOS kernel to provide a syncing folder was neither sustainable nor secure
> in the long term."
> — 9to5Mac, *From rogue Dropbox folders to the File Provider framework*
> https://9to5mac.com/2026/03/21/apple-work-from-rogue-dropbox-folders-to-the-file-provider-framework/

> "The software shoehorned its way into macOS using custom kernel extensions to add sync
> icons directly into Finder."
> — same source

> "Apple recognized this and introduced the File Provider framework. This framework gave
> cloud storage providers a native, secure, and standardized way to integrate directly into
> Finder without compromising the operating system."
> — same source

Apple's own DTS engineer ("Quinn") on the *multi-year* nature of this migration:

> "No concrete deadline has been announced. Moving away from KEXTs is a long-term process.
> Some KEXT technologies have already been removed, some still work but have been deprecated
> in favour of a user-space alternative, and some are still waiting for their user-space
> alternative to be announced."
> — Quinn "The Eskimo!", Apple DTS, https://developer.apple.com/forums/thread/681325

> "The situation with VFS plug-ins is nuanced. Many of the existing VFS plug-ins can be
> replaced by a file provider but, as you've pointed out, that's not universally true."
> — same source

**The lesson:** This is not a "build it once" feature. Dropbox built a kernel extension,
then had to **re-platform the entire thing onto File Provider** (their folder even moved to
`~/Library/CloudStorage`, breaking links in third-party apps — see §2.4). Building a
placeholder FS means signing up to track and re-implement against a *moving* OS target,
indefinitely. Sync durability itself is hard at scale:

> "Bidirectional sync has many corner cases, and durability is harder than just making sure
> we don't delete or corrupt data on the server."
> — Dropbox, *Rewriting the heart of our sync engine*
> https://dropbox.tech/infrastructure/rewriting-the-heart-of-our-sync-engine

> "Guaranteeing durability in a particular environment requires understanding its
> implementation, mitigating its bugs, and sometimes even reverse-engineering it when
> debugging production issues."
> — same source

> "There's enormous variation in hardware, and users also install different kernel
> extensions or drivers that change the behavior within the operating system."
> — same source

---

## 2. Specific app-compatibility failure modes that bite placeholder filesystems

These are the concrete ways apps break when a remote file is presented as if it were a
normal local file. **This is the core risk of option (A).**

### 2.1 Partial / random I/O — apps read or write *parts* of files

Most VFS APIs are all-or-nothing: a file is either fully "in the cloud" or fully
downloaded. Apps that read just a slice (a thumbnail, a DB page, a video keyframe) force a
full download — or simply fail. From Apple's developer forums (a developer migrating a real
VFS kext to File Provider, *not contradicted by Apple staff in-thread*):

> "Files can either be in the cloud or downloaded. It is not possible to download/read only
> a portion of a file. This is a big performance problem for us, since we work with large
> images (> 1GB). The programs we integrate with typically only read a part of the image,
> e.g. the embedded preview. The API does not offer a way to access selected blocks of a
> file (random access file)."
> — developer "_Michael_", https://developer.apple.com/forums/thread/681325

The same all-or-nothing limit shows up in user-space mounts. rclone's mount docs:

> "Without the use of `--vfs-cache-mode` this can only write files sequentially, it can only
> seek when reading."
> — https://rclone.org/commands/rclone_mount/

> "Many applications won't work with their files on an rclone mount without
> `--vfs-cache-mode writes` or `--vfs-cache-mode full`."
> — same source

> "Files can't be opened for both read AND write" … "Files opened for write can't be seeked."
> (`--vfs-cache-mode off`)
> — same source

The fix rclone uses is telling: to support normal app I/O you must **download the whole file
to a local cache** (i.e., you converge toward option B anyway).

### 2.2 Atomic-save / "write temp then rename" — the Office & VS Code pattern

The single most important compatibility pattern: editors do **not** write in place. They
write a new temp file, fsync it, then `rename()` it over the original (atomic replace). This
keeps data safe across crashes — but it hammers a placeholder FS, because the "rename over
original" interacts badly with sync locking, and the new file is a *brand-new inode* the
sync engine must re-upload wholesale.

> "A common pattern when updating a file is to write out a new version with a different name
> and then rename it over the existing file using an atomic rename operation."
> — microhowto, http://www.microhowto.info/howto/atomically_rewrite_the_content_of_a_file.html

> "Atomic rename operations across filesystem boundaries are not possible … a rename can be
> atomic [only] if source and target files share the same directory."
> — same source

When the underlying mount can't honor atomic rename on open/locked files, real apps break.
Documented in the wild on a Google Drive virtual mount:

> "GVfs Google Drive mount fails atomic file operations (rename/delete), breaking apps like
> KeePassXC with I/O errors"
> — https://github.com/linuxmint/cinnamon/issues/13555

And the performance cost of round-tripping these saves through hydration:

> "Saving even a small MS Office document will become very slow, not saying about AutoCAD or
> Photoshop files."
> — userfilesystem.com (Windows Cloud Files API FAQ), https://www.userfilesystem.com/programming/faq/

> "Applications accessing file system expect fast response and make a lot of reads and
> writes. Any delays will make the file system very slow and unusable."
> — same source

### 2.3 Latency surfacing as UI hangs, beachballs, and deadlocks

A remote read can take seconds. Apps issue file calls **synchronously on the main thread**,
assuming local-disk latency — so a placeholder read turns into a frozen UI. Apple's own
guidance (TN3150, *dataless files*):

> "In a modern file system, a file's content may not be available locally on the device. A
> file that contains only metadata is known as a _dataless_ file."
> — Apple TN3150 (via https://mjtsai.com/blog/2023/05/11/getting-ready-for-dataless-files/)

> "avoid unnecessarily materializing dataless files and, when your app requires access to a
> file's contents, perform that work asynchronously off the main thread."
> — Apple TN3150 (same)

> "Even an action as simple as checking whether a file exists can now take an unexpectedly
> long amount of time."
> — Michael Tsai, https://mjtsai.com/blog/2023/05/11/getting-ready-for-dataless-files/

> "the `NSFileCoordinator` APIs are awkward, error-prone, and slow, and they infect your
> entire codebase." … "Any file-related code could potentially need special handling, but
> there's no way to make sure that you didn't miss a spot somewhere."
> — Michael Tsai (same)

The deadlock failure is real and recent. A bug report against **Claude Code** shows what
happens when a process reads a dataless File-Provider file without participating in the
coordination protocol:

> "When a user mounts a folder that resides on a macOS FileProvider-backed filesystem (e.g.,
> Google Drive, iCloud Drive, Dropbox, OneDrive), … all attempts to read file contents fail
> with `EDEADLK` (errno 35, "Resource deadlock avoided")."
> — https://github.com/anthropics/claude-code/issues/40783

> "Every method of reading file contents — `cat`, `cp`, `dd`, Python `os.read()`, Node.js
> `fs.readFileSync()` — fails with the same errno 35."
> — same source

> "macOS FileProvider is a virtual filesystem where files can be \"dataless\" (dehydrated).
> The directory entry and metadata are cached locally, but the actual file content resides
> on the cloud provider's servers and is only downloaded (\"materialized\") on demand when a
> process reads the file."
> — same source

> "`Blocks: 0` is a telltale sign of a dataless/dehydrated FileProvider file."
> — same source

Apple's File-Provider *daemon itself* has shipped deadlocks:

> "FileProvider Daemon (fileproviderd) hangs when NSFileProviderManager.add is called twice
> within one second. … It looks like a deadlock."
> — https://developer.apple.com/forums/thread/715229 (fixed only in macOS 13)

### 2.4 Enumeration, paths, packages, mmap, and offline behavior

**Enumeration requirement** — a placeholder FS must list contents before they're
accessible, which clashes with dynamic/lazy backends (exactly what an SFTP/Drive explorer
is):

> "The File Provider learns about the file system content via `enumerators`. So everything
> that is inside a folder must be enumerated (listed) first. Otherwise it cannot be
> accessed. However, we cannot enumerate our VFS. Most of the content of our VFS is fully
> dynamic. It only exists when it is accessed by a client the first time."
> — developer "_Michael_", https://developer.apple.com/forums/thread/681325

**Paths break and some app classes are simply disallowed** — Dropbox's File Provider
migration *moved the folder and broke third-party links*:

> "Due to the change of the Dropbox folder location, files that were previously linked in
> some third-party applications will need to be linked again." … "Your Dropbox folder will
> be moved to _~/Library/CloudStorage._"
> — Dropbox Help, https://help.dropbox.com/installs/macos-support-for-expected-changes

> "Final Cut Pro libraries can't be saved or opened in the Dropbox folder." … "Apple no
> longer allows Photos Library to be synced over cloud-storage services."
> — same source

> "New macOS packages appear as regular files, but are actually bundles that contain other
> file types within."
> — same source (app bundles / `.app` / `.key` packages are a known sharp edge)

**Why the previous (placeholder/system-extension) model needed offline copies at all** —
Dropbox itself documents that before File Provider, third-party apps frequently *couldn't*
use online-only files:

> "If you're not on Dropbox on File Provider, you may need to make files available offline in
> order to use them in some third-party applications."
> — Dropbox Help (online-only files guidance)

**mmap / memory-mapped I/O** — apps (SQLite, media tools, some editors) `mmap()` files and
expect page-fault-backed access to real bytes. A placeholder file has *no data blocks*
(`Blocks: 0`, per §2.3); mapping it requires full materialization and breaks the assumption
of cheap, lazy, random page access — another reason all-or-nothing hydration is forced.

---

## 3. What these teams concluded about when the complexity is worth it

- **Dropbox** concluded the complexity is worth it for them because their *entire product*
  is "make your whole cloud appear as native local files for every app." Even so, they paid
  twice: a kernel extension first, then a full re-platform to File Provider. Their sync blog
  is candid that the problem is fundamentally hard:
  > "Syncing files becomes much harder at scale, and understanding why is important for
  > understanding why we decided to rewrite."
  > — https://dropbox.tech/infrastructure/rewriting-the-heart-of-our-sync-engine
  > "The number of possible combinations of file states and user actions is astronomical."
  > — same source

- **Apple** concluded that third parties should *not* write VFS kernel code at all, and
  funneled everyone into File Provider — accepting that File Provider deliberately **does
  not** support some VFS use cases (partial reads, fully-dynamic enumeration). Apple's
  guidance to developers hitting those walls is essentially "file a bug and wait":
  > "My advice is that you file bugs about all the sticking points you encounter. That's the
  > best way to ensure that the team working on this understands your unique requirements
  > before they finally pull the plug … on VFS plug-ins."
  > — Quinn, Apple DTS, https://developer.apple.com/forums/thread/681325

- **Sync vendors (rclone, ownCloud/Nextcloud, userfilesystem.com)** converge on the same
  practical answer: to make ordinary apps work, you must **fall back to a real local cache
  of the whole file** (rclone's `--vfs-cache-mode full`; CfApi hydration on open). At that
  point the "virtual" FS is mostly a download-on-demand cache — i.e., option (B) with extra
  OS-integration machinery bolted on. rclone states the underlying mismatch plainly:
  > "File systems expect things to be 100% reliable, whereas cloud storage systems are a
  > long way from 100% reliable."
  > — https://rclone.org/commands/rclone_mount/

**Net conclusion across all three:** A placeholder VFS is worth it *only* when (a) you are an
always-running platform, (b) you must satisfy arbitrary third-party apps transparently, and
(c) you can fund continuous OS-version maintenance and a real local cache for I/O
correctness. Absent any of those, the complexity is not worth it.

---

## 4. Lessons for a small file explorer that is NOT an always-running sync platform

Smart Explorer is a *user-driven* explorer that opens/edits specific files — not a daemon
that must make a whole backend look native to every app on the machine. The evidence above
maps almost perfectly onto **recommending option (B): download a real copy, edit, save
back** for the primary path.

1. **You don't owe transparency to arbitrary apps.** The entire VFS pain budget exists to
   make *unmodified third-party apps* treat remote files as local. A file explorer controls
   the open/edit flow itself, so it can hand the editor a **real, fully-materialized local
   temp file** — sidestepping partial-I/O, mmap, enumeration, and dataless-file hazards
   wholesale (§2.1, §2.4).

2. **Atomic-save just works on a real local file.** Editors that "write temp then rename"
   (Office, VS Code, vim, etc.) behave correctly on a normal local temp copy. Detect the
   save via inode/rename + mtime/size change, then **upload the resulting bytes**. Do not
   try to make rename-over semantics survive a remote round-trip in real time (§2.2). Watch
   for the inode/path changing on save — the saved file may be a *new* inode, so watch the
   directory, not just the original file handle.

3. **Latency stays off the critical path.** Download with a visible progress UI *before*
   handing the file to the editor; upload on save with progress/retry. This avoids the
   "synchronous read on the main thread → beachball/`EDEADLK`" trap that bites placeholder
   filesystems (§2.3). Never let a network round-trip happen inside a `read()`/`stat()` an
   app issues synchronously.

4. **Offline and conflict handling become explicit and simple.** With a real local copy you
   have a concrete artifact to retry, diff, or save-as-conflict if the upload fails or the
   remote changed underneath you. A placeholder FS must answer "what does `read()` return
   when offline?" for *every* syscall; option (B) only answers it at explicit
   download/upload boundaries.

5. **No kernel code, no File Provider extension, no perpetual OS-version treadmill.** Dropbox
   shipped a kernel extension and then had to re-platform onto File Provider (folder moved to
   `~/Library/CloudStorage`, links broke — §1.2, §2.4). A cross-platform Rust explorer should
   not sign up for per-OS VFS integration (macFUSE/File Provider on macOS, CfApi on Windows,
   FUSE on Linux) unless transparent always-on mounting is a core product requirement.

6. **If you ever *do* want "appears in Finder/Explorer," treat it as a separate, later,
   per-OS project** built on the *sanctioned* APIs (File Provider on macOS, Cloud Files API
   on Windows) — and budget for a real local cache regardless, because every vendor above had
   to fall back to caching whole files to make apps work (§3). Don't reach for FUSE/macFUSE
   kexts: they are deprecated on Apple Silicon and were the exact thing Dropbox and Apple
   moved *away* from.

**Recommendation:** Default to **(B) download-real-copy / save-back**. It eliminates the
documented failure modes (atomic-save rename, partial I/O, mmap, latency-hangs, enumeration,
offline semantics) that cost Dropbox a kernel extension and a full re-platform, and that Apple
deliberately refuses to fully support even in File Provider. Reserve a placeholder/mount-style
VFS for a future, optional, per-OS "mount as drive" feature built on sanctioned user-space
APIs with a whole-file cache — never as the core architecture.

---

## Sources (all URLs)

- Dropbox, *Going deeper with Project Infinite* — https://dropbox.tech/infrastructure/going-deeper-with-project-infinite
- Dropbox, *Rewriting the heart of our sync engine* — https://dropbox.tech/infrastructure/rewriting-the-heart-of-our-sync-engine
- Michael Tsai, *Dropbox's Upcoming Kernel Extension* — https://mjtsai.com/blog/2016/05/26/dropboxs-upcoming-kernel-extension/
- Michael Tsai, *Getting Ready for Dataless Files* (quotes Apple TN3150) — https://mjtsai.com/blog/2023/05/11/getting-ready-for-dataless-files/
- Apple Developer Forums, *Alternative for VFS kernel extension* (Quinn/DTS + partial-read & enumeration limits) — https://developer.apple.com/forums/thread/681325
- Apple Developer Forums, *fileproviderd hangs / deadlock* — https://developer.apple.com/forums/thread/715229
- 9to5Mac, *From rogue Dropbox folders to the File Provider framework* — https://9to5mac.com/2026/03/21/apple-work-from-rogue-dropbox-folders-to-the-file-provider-framework/
- Dropbox Help, *Expected changes with Dropbox for macOS on File Provider* — https://help.dropbox.com/installs/macos-support-for-expected-changes
- GitHub, claude-code #40783, *File read failure on macOS FileProvider-backed paths (EDEADLK)* — https://github.com/anthropics/claude-code/issues/40783
- rclone, *rclone mount* docs (VFS cache modes, sequential/seek limits, reliability) — https://rclone.org/commands/rclone_mount/
- userfilesystem.com, *Cloud Files API FAQ* (MS Office save latency, hydration) — https://www.userfilesystem.com/programming/faq/
- GitHub, cinnamon #13555, *GVfs Google Drive mount fails atomic file operations (KeePassXC)* — https://github.com/linuxmint/cinnamon/issues/13555
- microhowto, *Atomically rewrite the content of a file* — http://www.microhowto.info/howto/atomically_rewrite_the_content_of_a_file.html
- Apriorit, *How to Work with the File Provider API on macOS* — https://www.apriorit.com/dev-blog/730-mac-how-to-work-with-the-file-provider-for-macos
