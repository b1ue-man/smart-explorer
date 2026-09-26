# Android: Kindprozesse von Apps – Kill-Mechanismus, Phantom Processes, Freezer, Seccomp/SELinux

Quelle: https://android.googlesource.com/platform/system/core/+/master/libprocessgroup/processgroup.cpp · https://android.googlesource.com/platform/system/core/+/master/libprocessgroup/include/processgroup/processgroup.h · https://github.com/aosp-mirror/platform_frameworks_base/blob/master/services/core/java/com/android/server/am/ProcessList.java · https://android.googlesource.com/platform/frameworks/base/+/master/services/core/java/com/android/server/am/PhantomProcessList.java · https://android.googlesource.com/platform/frameworks/base/+/master/services/core/java/com/android/server/am/ActivityManagerConstants.java · https://android.googlesource.com/platform/frameworks/base.git/+/09dcdad5ebc159861920f090e07da60fac71ac0a · https://source.android.com/docs/core/perf/cached-apps-freezer · https://source.android.com/docs/core/architecture/ipc/binder-freezer · https://source.android.com/docs/core/perf/cgroups · https://android.googlesource.com/platform/bionic/+/master/libc/SECCOMP_ALLOWLIST_COMMON.TXT · https://android.googlesource.com/platform/bionic/+/android-9.0.0_r33/libc/SECCOMP_BLACKLIST_APP.TXT · https://android-developers.googleblog.com/2017/07/seccomp-filter-in-android-o.html · https://man7.org/linux/man-pages/man5/proc.5.html · https://man7.org/linux/man-pages/man2/PR_SET_CHILD_SUBREAPER.2const.html · https://github.com/aosp-mirror/platform_system_core/commit/c39ba5ae32afb6329d42e61d2941d87ff66d92e3 · https://developer.android.com/guide/components/activities/process-lifecycle · https://developer.android.com/about/versions/14/behavior-changes-all · Abgerufen: 2026-09-26

## 1. Kill aller vom App-Prozess geforkten Prozesse beim Tod des Apps

Der native Mechanismus ist `killProcessGroup(uid, initialPid, signal)` in `libprocessgroup` (`system/core/libprocessgroup/processgroup.cpp`, AOSP `master`):

```
int killProcessGroup(uid_t uid, pid_t initialPid, int signal) {
    return KillProcessGroup(uid, initialPid, signal);
}
```

Der Header `system/core/libprocessgroup/include/processgroup/processgroup.h` dokumentiert den Rückgabewert: "Return 0 if all processes were killed and the cgroup was successfully removed. Returns -1 in the case of an error occurring or if there are processes still running."

Intern ermittelt `KillProcessGroup` zuerst den cgroup-v2-Pfad über `ConvertUidPidToPath(hierarchy_root_path, uid, initialPid, true)` – das ist exakt das Namensschema `uid_<uid>/pid_<pid>`, das auch `source.android.com/docs/core/perf/cgroups` für die per-App-cgroup-Hierarchie beschreibt (Abstraktionsschicht `cgroups.json`/`task_profiles.json`, laut dieser Seite mit APIs "available in Android 10+"; das genaue Einführungsdatum des `killProcessGroup`-Mechanismus selbst ließ sich aus den abgerufenen Seiten nicht weiter zurückverfolgen – siehe Offene Punkte).

Zum eigentlichen Töten gibt es zwei Pfade:
- Bei `SIGKILL` und wenn `CgroupKillAvailable()` wahr ist, schreibt der Code `"1"` in die Datei `cgroup.kill` der cgroup, was laut Kommentar im Code atomar alle Prozesse der cgroup killt.
- Andernfalls (andere Signale, oder als Fallback) liest der Code `cgroup.procs` der cgroup aus und signalisiert jede dort enthaltene PID einzeln; Prozessgruppen-Leader werden zusätzlich per `kill(-pgid, signal)` behandelt. Kommentar im Code: "Since cgroup.kill only sends SIGKILLs, we read cgroup.procs to find each process to signal individually. This is more costly than using cgroup.kill for SIGKILLs."

Aufgerufen wird das Ganze aus dem Java-Framework über `ProcessList.killProcessGroup(int uid, int pid)` in `services/core/java/com/android/server/am/ProcessList.java` (AOSP `master`), das die Arbeit an einen dedizierten `KillHandler`-Thread delegiert:

```java
static void killProcessGroup(int uid, int pid) {
    /* static; one-time init here */
    if (sKillHandler != null) {
        sKillHandler.sendMessage(
                sKillHandler.obtainMessage(KillHandler.KILL_PROCESS_GROUP_MSG, uid, pid));
```

```java
case KILL_PROCESS_GROUP_MSG:
    Trace.traceBegin(Trace.TRACE_TAG_ACTIVITY_MANAGER, "killProcessGroup");
    Process.killProcessGroup(msg.arg1 /* uid */, msg.arg2 /* pid */);
    Trace.traceEnd(Trace.TRACE_TAG_ACTIVITY_MANAGER);
    break;
```

`Process.killProcessGroup` ist die native JNI-Brücke zu `libprocessgroup`. Die konkreten Aufrufstellen von `ProcessList.killProcessGroup(uid, pid)` (Prozesstod, Force-Stop, LMKD-Kill) ließen sich innerhalb der im Rahmen dieser Recherche abgerufenen Ausschnitte von `ProcessList.java` nicht mit Zeilennummer belegen (die Datei ist sehr groß); dass ProcessList/ActivityManagerService diesen Aufruf beim Entfernen eines toten App-Prozesses tätigt, ist durch den Nachrichtennamen (`KILL_PROCESS_GROUP_MSG`) und die etablierte Funktion von `libprocessgroup` als "Cleanup nach App-Tod" hinreichend belegt, aber die exakte Aufruf-Zeile ist damit **(Schluss)**, kein wörtliches Zitat.

**Antwort auf die Kernfrage (Schluss, aber direkt aus den obigen Primärquellen abgeleitet):** Ja – auch Prozesse, die `setsid()` aufgerufen haben oder sich "doppelt geforkt" haben, werden mitgekillt. Grund: Die cgroup-Mitgliedschaft eines Prozesses wird beim `fork()`/`clone()` vom Elternprozess vererbt und bleibt unabhängig von Session- oder Prozessgruppen-Wechseln (`setsid()` ändert nur Session-ID/Prozessgruppe, nicht die cgroup) erhalten, solange der Prozess sich nicht explizit selbst in eine andere cgroup verschiebt (dafür fehlen einer `untrusted_app`-App die nötigen Rechte). Da `killProcessGroup` über `cgroup.kill`/`cgroup.procs` **alle** PIDs der `uid_<uid>/pid_<pid>`-cgroup erfasst – unabhängig von Session/Prozessgruppe –, werden double-forked/`setsid()`-Kinder mit erfasst, solange sie in derselben cgroup verblieben sind.

## 2. Phantom Processes (Android 12+)

Der Grenzwert ist in `services/core/java/com/android/server/am/ActivityManagerConstants.java` (AOSP `master`) definiert:

```java
private static final int DEFAULT_MAX_PHANTOM_PROCESSES = 32;
...
private static final String KEY_MAX_PHANTOM_PROCESSES = "max_phantom_processes";
...
public int MAX_PHANTOM_PROCESSES = DEFAULT_MAX_PHANTOM_PROCESSES;
```

Der Default ist also **32, systemweit** (über alle Apps hinweg, nicht pro App), konfigurierbar per `device_config` unter dem Schlüssel `max_phantom_processes`.

Die eigentliche Verwaltung/Erkennung übernimmt `PhantomProcessList.java` (`services/core/java/com/android/server/am/PhantomProcessList.java`, AOSP `master`); Zeile 44 trägt den Kommentar "Activity manager code dealing with phantom processes", die Trimm-Logik greift laut Code auf `mService.mConstants.MAX_PHANTOM_PROCESSES` zurück, wenn die Zahl überschritten wird.

Getötet werden Phantom-Prozesse laut den ausgewerteten Sekundärquellen (AOSP-Committer-Aussagen/XDA-Berichterstattung zur stillen Einführung in Android 12) dann, wenn entweder mehr als `MAX_PHANTOM_PROCESSES` gleichzeitig laufen **oder** ein Phantom-Prozess exzessive CPU verbraucht, während sein Eltern-App-Prozess selbst im Hintergrund ist. Ein "Phantom-Prozess" ist dabei jeder Prozess, der von einem App-Prozess abgezweigt wurde und dessen Lebenszyklus das Framework nicht über die üblichen Service/Activity-Mechanismen kennt (klassisch: `Runtime.exec()`/`ProcessBuilder`).

Die Umschalt-Möglichkeit wurde per Commit `09dcdad5ebc159861920f090e07da60fac71ac0a` in `platform/frameworks/base` eingeführt (Commit-Message: "Add settings to toggle the phantom process monitoring in dev options. For power users, the monitoring on phantom processes could be turned off from the Settings->Developer Options->Feature flags."), betroffene Dateien laut Diff: `core/java/android/util/FeatureFlagUtils.java`, `services/core/java/com/android/server/am/ActivityManagerService.java`, `services/core/java/com/android/server/am/AppProfiler.java`, `services/core/java/com/android/server/am/PhantomProcessList.java`. Dieser Commit führt das Feature-Flag `settings_enable_monitor_phantom_procs` ein (zunächst nur über Settings → Developer Options → Feature Flags erreichbar, laut zeitgleicher Berichterstattung ab Android 12L Beta 3 verfügbar). In späteren Releases (spätestens ab der von Mishaal Rahman im Februar 2023 dokumentierten Version, danach in Android 14 durchgehend vorhanden) gibt es zusätzlich einen direkten Schalter "Disable child process restrictions" unter Einstellungen → System → Entwickleroptionen, der laut dieser Quelle "disables the monitoring of so-called 'phantom processes', child processes forked by app processes that the framework couldn't track the lifecycle of" **(Sekundärquelle X/Threadreader, nicht AOSP-Primärquelle für den UI-Text selbst; der zugrunde liegende Flag-Mechanismus ist aber durch den obigen AOSP-Commit primärquellenbelegt)**.

## 3. Cached Apps Freezer (Android 11+/12+)

`source.android.com/docs/core/perf/cached-apps-freezer` (wörtliche Zitate):

> "This feature stops execution for cached processes and reduces resource usage by misbehaving apps that might attempt to operate while cached."

> "Android 11 (API level 30) or higher supports the cached apps freezer."

> "Android freezes cached apps by migrating their processes into a frozen cgroup."

`source.android.com/docs/core/architecture/ipc/binder-freezer` (wörtliches Zitat):

> "When an app has no user-visible components, such as activities or services, it can be moved to the _cached_ state."

**Einordnung (Schluss):** Die zweite Quelle definiert den "cached"-Zustand explizit über das Fehlen sichtbarer Komponenten wie Activities **oder Services**. Ein Foreground Service ist eine laufende `Service`-Komponente (mit Nutzer-sichtbarer Notification) – ein App-Prozess mit aktivem Foreground Service erfüllt die Bedingung "no user-visible components […] such as […] services" nicht und wird deshalb nicht in den `cached`-Zustand versetzt, also **nicht** vom Freezer eingefroren. Eine explizite Ausnahme-Formulierung ("foreground service is never frozen") ließ sich in den abgerufenen Primärquellen nicht wörtlich finden; das ist eine Ableitung aus der zitierten Zustandsdefinition, keine wörtliche Aussage.

**Ob geforkte Kindprozesse mitgefroren werden (Schluss):** Die erste Quelle spricht von "migrating **their** processes into **a** frozen cgroup" (Plural "processes", Singular "a … cgroup" pro App). Kombiniert mit dem in Abschnitt 1 belegten `uid_<uid>/pid_<pid>`-cgroup-Schema und dem allgemeinen Linux-cgroup-Verhalten (ein `fork()`/`clone()`-Kind verbleibt standardmäßig in der cgroup seines Elternprozesses, sofern es nicht explizit umzieht) folgt: Ja, ein per `fork`/`clone` erzeugtes Kind eines eingefrorenen (cached) App-Prozesses wird mit eingefroren, weil es sich in derselben cgroup befindet, die der Freezer einfriert. Ein wörtlicher AOSP-Satz, der Kindprozesse explizit nennt, wurde in den abgerufenen Ausschnitten nicht gefunden (siehe Offene Punkte).

Zusatz aus Android 14 (`developer.android.com/about/versions/14/behavior-changes-all`, wörtliches Zitat, thematisch verwandt – "cached state" wird ab Android 14 strenger durchgesetzt):

> "Android 14 introduces consistency and enforcement to this design. Shortly after an app process enters a cached state, background work is disallowed, until a process component re-enters an active state of the lifecycle."

## 4. Seccomp/SELinux/`/proc`: sind `prctl`, `clone`, `/proc`-Zugriff und `exec` von `sh`/toybox für `untrusted_app` erlaubt?

**Seccomp – `prctl`/`clone`/`clone3`:** Laut dem Android-O-Blogpost ("Seccomp filter in Android O", android-developers.googleblog.com) installiert Zygote genau einen Seccomp-Filter für alle App-Prozesse; der Filter erlaubt grundsätzlich "all the syscalls exposed via bionic" (definiert in `bionic/libc/SYSCALLS.TXT`) und blockiert daraus gezielt eine Teilmenge ("the filter blocks 17 of 271 syscalls in arm64 and 70 of 364 in arm", u. a. `swapon`/`swapoff` und "the key control syscalls").

Die aktuelle App-Blockliste `bionic/libc/SECCOMP_BLACKLIST_APP.TXT` (Android-9-Tag als abgerufene Version) listet u. a. `setgid`, `setuid`, `setreuid`, `setresgid`, `setfsgid`, `setfsuid`, `setgroups`, `adjtimex`, `clock_adjtime`, `clock_settime`, `settimeofday`, `acct`, `klogctl`, `chroot`, `init_module`, `delete_module`, `mount`, `umount2`, `swapon`, `swapoff`, `setdomainname`, `sethostname`, `reboot` – **`prctl` und `clone` stehen nicht auf dieser Blockliste**. Die zusätzliche Allowlist `bionic/libc/SECCOMP_ALLOWLIST_COMMON.TXT` (AOSP `master`) listet `clone` und `clone3` explizit:

- `clone(int (*)(void*), void*, int, void*, ...) all` – mit Kommentar, dass `vfork` von bionic (und `java.lang.ProcessBuilder`) auf manchen Architekturen genutzt wird, andere Architekturen nutzen `clone(2)` direkt.
- `clone3(clone_args*, size_t) all`

Kopfkommentar der Datei: "This file is used to populate seccomp's allowlist policy in combination with SYSCALLS.TXT. Note that the resultant policy is applied only to zygote spawned processes." – also genau die App-Prozesse (inkl. `untrusted_app`).

**Fazit `prctl`:** `prctl` ist ein regulärer, über bionic exponierter Syscall (`SYSCALLS.TXT`) und erscheint auf keiner der eingesehenen Block-/Denylisten für App-Prozesse; da laut Blogpost die Baseline "allow" ist und nur eine explizite Teilmenge geblockt wird, ist `prctl` – und damit `prctl(PR_SET_CHILD_SUBREAPER, ...)` – für `untrusted_app`-Prozesse zulässig **(Schluss, gestützt auf die Abwesenheit von `prctl` in der Blockliste plus die dokumentierte "allow-by-default"-Policy)**. Ein raw `clone()` mit `SIGCHLD` ist durch den expliziten Allowlist-Eintrag für `clone(...)` direkt primärquellenbelegt.

**`PR_SET_CHILD_SUBREAPER` (man7.org, `prctl(2)`):**

> "If _set_ is nonzero, set the 'child subreaper' attribute of the calling process; if _set_ is zero, unset the attribute." … "A subreaper fulfills the role of init(1) for its descendant processes. When a process becomes orphaned (i.e., its immediate parent terminates), then that process will be reparented to the nearest still living ancestor subreaper."

**`/proc`-Sichtbarkeit (`hidepid`/`gid=3009`):** AOSP-Commit `c39ba5ae32afb6329d42e61d2941d87ff66d92e3` ("Enable hidepid=2 on /proc", `platform/system/core`) mountet `/proc` mit `hidepid=2,gid=3009` (`AID_READPROC`). Laut `proc(5)` (man7.org):

> hidepid=1: "Users may not access files and subdirectories inside any /proc/pid directories but their own (the /proc/pid directories themselves remain visible)."
> hidepid=2: "As for mode 1, but in addition the /proc/pid directories belonging to other users become invisible."
> `gid=`: "Specifies the ID of a group whose members are authorized to learn process information otherwise prohibited by hidepid (i.e., users in this group behave as though /proc was mounted with hidepid=0)."

**Fazit `/proc`:** Da ein von der App per `clone()` erzeugtes Kind dieselbe Linux-UID trägt wie der App-Prozess selbst, zählt es aus Sicht von `hidepid` als "own" – `/proc/<pid>/stat` und `/proc/<pid>/cmdline` der eigenen Kindprozesse bleiben also auch unter `hidepid=2` für die App lesbar, ohne dass die App Mitglied der Gruppe `AID_READPROC` (3009) sein müsste **(Schluss, direkt aus der zitierten hidepid-Semantik + gleicher UID)**. Für fremde UIDs (andere Apps) gilt die Sperre.

**`exec` von `/system/bin/sh` und Toybox-Tools unter SELinux:** Für den konkreten SELinux-Regelsatz (`exec_type`, `shell_exec`, `toolbox_exec`) ließ sich im Rahmen dieser Recherche keine wörtliche, primärquellenbelegte Freigaberegel für `untrusted_app` extrahieren (die eingesehenen Ausschnitte von `public/domain.te` und `public/te_macros` zeigten `neverallow`-Beschränkungen für `execute_no_trans` primär gegenüber `init`/Kern-Domains, nicht die konkrete Freigabe für Apps). Als Kontext gefunden: Die dokumentierte Android-10+-Härtung ("W^X") verweigert `untrusted_app` per SELinux gezielt das `execve()` von Dateien mit dem Label `app_data_file` – also von Binaries, die die App selbst in ihr eigenes (beschreibbares) Datenverzeichnis abgelegt hat; das betrifft **nicht** System-Binaries wie `/system/bin/sh`. Dass `Runtime.exec()`/`ProcessBuilder` mit System-Binaries (`sh`, toybox-Tools) auf Stock-Android für normale Drittanbieter-Apps grundsätzlich funktioniert (nicht per SELinux pauschal verweigert wird), ist durch die extrem verbreitete Praxis entsprechender Root-Check-, Diagnose- und Terminal-Apps sowie durch das Fehlen eines dokumentierten generischen `neverallow untrusted_app shell_exec:file execute` in den eingesehenen Ausschnitten gestützt, aber **(Schluss)** – kein wörtliches "allow"-Zitat für genau diesen Fall. Siehe Offene Punkte.

## 5. Guidance/Pitfalls für langlaufende Kindprozesse (Android 14–16)

`developer.android.com/guide/components/activities/process-lifecycle` (wörtliche Zitate):

> "An unusual and fundamental feature of Android is that an application process's lifetime _isn't_ directly controlled by the application itself. Instead, it is determined by the system through a combination of the parts of the application that the system knows are running, how important these things are to the user, and how much overall memory is available in the system."

> "So, the system can kill the process at any time to reclaim memory, and in doing so, it terminates the spawned thread running in the process."

> "A cached process is one that is not currently needed, so the system is free to kill it as needed when resources like memory are needed elsewhere." … "Cached processes can be killed by the system at any time, apps should cease all work while in the cached state."

> "Be aware that `onDestroy()` is not guaranteed to be called in the case that a process is killed by the system."

> "Starting in Android 13, an app process may receive limited or no execution time until it enters one of the above active lifecycle states."

`developer.android.com/about/versions/14/behavior-changes-all` (wörtliche Zitate):

> "Starting in Android 14, when your app calls `killBackgroundProcesses()`, the API can kill only the background processes of your own app. If you pass in the package name of another app, this method has no effect on that app's background processes, and the following message appears in Logcat: Invalid packageName: com.example.anotherapp"

> "Your app shouldn't use the `killBackgroundProcesses()` API or otherwise attempt to influence the process lifecycle of other apps, even on older OS versions."

> "By design, an app's process is in a cached state when it's moved to the background and no other app process components are running. Such an app process is subject to being killed due to system memory pressure. Any work that `Activity` instances perform after the `onStop()` method has been called and returned, while in this state, is unreliable and strongly discouraged. Android 14 introduces consistency and enforcement to this design. Shortly after an app process enters a cached state, background work is disallowed, until a process component re-enters an active state of the lifecycle. Apps that use typical framework-supported lifecycle APIs – such as services, `JobScheduler`, and Jetpack WorkManager – shouldn't be impacted by these changes."

**Einordnung für unseren Anwendungsfall (Schluss):** Für Android 14–16 gilt in Summe: (a) Ein App-Prozess selbst kann jederzeit gekillt werden, sobald er "cached" ist (keine sichtbare Activity/Service mehr) – dann werden per `killProcessGroup` auch alle seine (Groß-)Kindprozesse mitgekillt (Abschnitt 1). (b) Phantom-Process-Killing (Abschnitt 2) und ab Android 14 die verschärfte "cached state"-Durchsetzung (Abschnitt 5) betreffen zusätzlich Kindprozesse, die *nicht* durch den Tod des Elternprozesses beendet werden, sondern eigenständig laufen, während der Eltern-App-Prozess im Hintergrund/cached ist. Ein Zwischenprozess mit `PR_SET_CHILD_SUBREAPER`, der Nachfahren am Leben hält, schützt also **nicht** vor (a), (b) oder dem Freezer (Abschnitt 3) – es sind drei unabhängige Kill-/Freeze-Pfade, die alle am App-Prozess bzw. an dessen cgroup ansetzen, nicht an Session/Prozessgruppe. Eine Aussage speziell für Android 15/16 (jenseits der zitierten Android-13/14-Änderungen) wurde in den abgerufenen Primärquellen nicht gefunden.

## Offene Punkte

- Die exakte Aufrufstelle (Datei + Zeile), an der `ProcessList.killProcessGroup(uid, pid)` beim Tod eines App-Prozesses aus `ActivityManagerService`/`ProcessRecord` heraus aufgerufen wird, wurde nicht mit Zeilennummer verifiziert (Datei sehr groß, nur Definition/Handler eingesehen).
- Das genaue Android-Release, in dem der cgroup-basierte `killProcessGroup`-Mechanismus selbst (nicht die `task_profiles.json`-Abstraktionsschicht) eingeführt wurde, ließ sich aus den abgerufenen Quellen nicht zweifelsfrei datieren.
- Kein wörtliches AOSP-Zitat gefunden, das explizit bestätigt, dass der Cached-Apps-Freezer *Kindprozesse* eines gefrorenen App-Prozesses einschließt; die Schlussfolgerung stützt sich auf cgroup-Vererbung plus die zitierte "uid_/pid_"-cgroup-Struktur.
- Kein wörtliches AOSP-Zitat gefunden, das eine explizite `neverallow`/`allow`-Regel für `untrusted_app`-Exec von `/system/bin/sh` bzw. Toybox-Binaries (Label `shell_exec`/`toolbox_exec`) zeigt; die Recherche in `public/domain.te` und `public/te_macros` blieb hier ohne eindeutigen Treffer (Tool-Limitierungen bei sehr großen `.te`-Dateien).
- Die genaue AOSP-Quelle für den UI-Text/-Zeitpunkt des Entwickleroptionen-Schalters "Disable child process restrictions" (Android 13 QPR vs. Android 14) wurde nur über eine Sekundärquelle (X/Threadreader, Mishaal Rahman, Februar 2023) belegt, nicht über einen eigenen AOSP-Commit-Diff mit Datum.
