"""Every other setting on every surface (settings parity batch, 2026-10-16).

Called only by native/test-luecken-task.py (`lib` is that module); no entry
point of its own. Plan: docs/plaene/2026-10-16-paritaet-einstellungen/plan.md,
surface inventory: docs/oberflaechen/einstellungen.md.

stage_settings  (Linux and Windows) the CLI side of the GUI settings in an
                isolated client: sync setups (the GUI's job file), background
                worker controls, saved connections (SMB, edit, removal with
                the GUI cleanup), Google Drive client, Explorer context menu,
                start-up update check. The GUIs are covered by the `unit`
                stage (desktop editor pages, connection dialog checks, plugin
                panel parser, bridge round trip) and the `android` job (JVM).
"""
import json
from pathlib import Path


def _json(completed):
    return json.loads(completed.stdout.decode(errors="replace"))


def _text(completed):
    return (completed.stdout + b"\n" + completed.stderr).decode(errors="replace")


def stage_settings(lib, results, se_binary, work, log_root):
    stage = "settings"
    logs = log_root / "settings"
    logs.mkdir(parents=True, exist_ok=True)
    log = logs / "settings.log"
    client = lib.Client(work, "settings")
    ensure = lib.ensure

    def se(*args, input_text=None):
        return client.se(se_binary, *args, input_text=input_text, log=log)

    side_a = Path(work) / "settings-a"
    side_b = Path(work) / "settings-b"
    side_a.mkdir(exist_ok=True)
    side_b.mkdir(exist_ok=True)

    def sync_setups():
        ensure(_json(se("sync", "list", "--json")) == [], "a fresh client lists sync setups")
        added = se("sync", "add", "--name", "LueckenDocs", "--source", str(side_a), "--target", str(side_b),
                   "--direction", "a2b", "--trigger", "calendar", "--cal-time", "02:30",
                   "--cal-weekdays", "mon,thu", "--ignore", "*.tmp", "--ignore", "cache/**",
                   "--bwlimit-kbps", "512", "--run-cleanup", "echo cleanup", "--versions-location", "appdata",
                   "--max-delete-min", "7", "--rt-poll-secs", "30", "--cross-mounts", "true", "--json")
        ensure(added.returncode == 0, _text(added))
        job = _json(added)
        expected = {"direction": "a2b", "trigger": "calendar", "cal_time": "02:30",
                    "cal_weekdays": ["mon", "thu"], "ignore": ["*.tmp", "cache/**"], "bwlimit_kbps": 512,
                    "run_cleanup": "echo cleanup", "versions_location": "appdata", "max_delete_min": 7,
                    "rt_poll_secs": 30, "cross_mounts": True, "enabled": True}
        for key, value in expected.items():
            ensure(job[key] == value, (key, job[key]))
        # The GUIs read exactly this file.
        conf = (client.app_data / "sync" / "jobs" / f"{job['id']}.conf").read_text(encoding="utf-8")
        for line in ("bwlimit_kbps=512", "run_cleanup=echo cleanup", "versions_location=appdata",
                     "cal_weekdays=9", "max_delete_min=7", "rt_poll_secs=30"):
            ensure(line in conf, f"{line} missing in the job file")
        mirror = se("sync", "edit", "LueckenDocs", "--direction", "both", "--delete-policy", "mirror")
        ensure(mirror.returncode != 0 and "Spiegel-Löschungen" in _text(mirror), _text(mirror))
        edited = se("sync", "edit", job["id"][:6], "--trigger", "realtime", "--rt-debounce-secs", "9", "--json")
        ensure(edited.returncode == 0, _text(edited))
        changed = _json(edited)
        ensure(changed["trigger"] == "realtime" and changed["rt_debounce_secs"] == 9, changed)
        ensure(changed["bwlimit_kbps"] == 512 and changed["ignore"] == ["*.tmp", "cache/**"], "edit lost settings")
        ensure(se("sync", "disable", "LueckenDocs").returncode == 0, "disable failed")
        ensure(_json(se("sync", "show", "LueckenDocs", "--json"))["enabled"] is False, "disable not stored")
        ensure(se("sync", "enable", "LueckenDocs").returncode == 0, "enable failed")
        verbose = se("sync", "log", "LueckenDocs", "--verbose", "true", "--json")
        ensure(verbose.returncode == 0 and _json(verbose)["verbose"] is True, _text(verbose))
        relative = se("sync", "add", "--source", "relative/dir", "--target", str(side_b))
        ensure(relative.returncode != 0 and "absolute" in _text(relative), _text(relative))
        removed = se("sync", "remove", "LueckenDocs")
        ensure(removed.returncode == 0, _text(removed))
        ensure(_json(se("sync", "list", "--json")) == [], "the setup was not removed")
    results.check(stage, "se sync: add/show/edit/disable/enable/log/remove on the GUI's job file", sync_setups)

    def background():
        status = _json(se("background", "status", "--json"))
        for key in ("sync_at_login", "paused", "check_interval_seconds", "autopause", "next_scheduled_run"):
            ensure(key in status, f"{key} missing")
        ensure(se("background", "cadence", "60").returncode == 0, "cadence 60 refused")
        ensure(_json(se("background", "status", "--json"))["check_interval_seconds"] == 60, "cadence not stored")
        ensure(se("background", "cadence", "1").returncode != 0, "cadence 1 accepted")
        ensure(se("background", "pause", "2h").returncode == 0, "pause 2h refused")
        paused = _json(se("background", "status", "--json"))
        ensure(paused["paused"] is True and 7000 < paused["pause_remaining_seconds"] <= 7200, paused)
        ensure(se("background", "pause", "forever").returncode == 0, "pause forever refused")
        ensure(_json(se("background", "status", "--json"))["paused_forever"] is True, "forever not stored")
        ensure(se("background", "resume").returncode == 0, "resume refused")
        ensure(_json(se("background", "status", "--json"))["paused"] is False, "resume not stored")
        support = status["autopause"]
        battery = se("background", "autopause", "--battery", "true", "--json")
        if support["battery_saver_supported"]:
            ensure(battery.returncode == 0 and _json(battery)["battery_saver"] is True, _text(battery))
            ensure(se("background", "autopause", "--battery", "false").returncode == 0, "battery off refused")
        else:
            ensure(battery.returncode != 0 and "not supported" in _text(battery), _text(battery))
        if not lib.WINDOWS:
            enabled = se("background", "enable")
            ensure(enabled.returncode == 0, _text(enabled))
            entry = client.root / "config" / "autostart" / "smart-explorer-sync-daemon.desktop"
            ensure(entry.is_file() and "--sync-daemon" in entry.read_text(), "no login entry")
            ensure(_json(se("background", "status", "--json"))["sync_at_login"] is True, "login start not shown")
            ensure(se("background", "disable").returncode == 0 and not entry.exists(), "login entry kept")
        return "autopause battery " + ("supported" if support["battery_saver_supported"] else "refused as unsupported")
    results.check(stage, "se background: cadence, pause, resume, autopause, login start like the GUI", background)

    def connections():
        saved = se("connections", "add", "smb", "--host", "nas.invalid", "--user", "WORK\\alice",
                   "--root", "/public/docs", "--label", "lueckennas", "--password-stdin", input_text="pw\n")
        ensure(saved.returncode == 0, _text(saved))
        bad = se("connections", "add", "smb", "--host", "nas.invalid", "--root", "/", "--label", "bad")
        ensure(bad.returncode != 0 and "share" in _text(bad), _text(bad))
        rows = [row for row in _json(se("connections", "list", "--json")) if row.get("label") == "lueckennas"]
        ensure(len(rows) == 1 and rows[0]["protocol"] == "smb" and rows[0]["auth"] == "password", rows)
        moved = se("connections", "edit", "lueckennas", "--host", "nas2.invalid")
        ensure(moved.returncode == 0 and "was smb://" in _text(moved), _text(moved))
        rows = [row for row in _json(se("connections", "list", "--json")) if row.get("label") == "lueckennas"]
        ensure(len(rows) == 1 and rows[0]["host"] == "nas2.invalid", rows)
        refused = se("connections", "edit", "lueckennas", "--key", "/k/id")
        ensure(refused.returncode != 0 and "sftp" in _text(refused), _text(refused))
        favorites = client.app_data / "favorites.txt"
        favorites.write_text("smb://WORK\\alice@nas2.invalid:445/public/docs/x\n/elsewhere\n", encoding="utf-8")
        removed = se("connections", "remove", "lueckennas")
        ensure(removed.returncode == 0 and "favorites removed=1" in _text(removed), _text(removed))
        ensure(favorites.read_text(encoding="utf-8").strip() == "/elsewhere", "the GUI cleanup did not run")
    results.check(stage, "se connections: add smb, edit (moved entry), remove with the GUI cleanup", connections)

    def google_drive():
        configured = se("connections", "gdrive", "configure", "--client-id", "luecken.apps.googleusercontent.com",
                        "--client-secret-stdin", input_text="s3cret\n")
        ensure(configured.returncode == 0, _text(configured))
        status = _json(se("connections", "gdrive", "status", "--json"))
        ensure(status == {"client_configured": True, "client_id": "luecken.apps.googleusercontent.com",
                          "signed_in": False}, status)
        ensure("s3cret" not in _text(se("connections", "gdrive", "status")), "the secret was printed")
        ensure(se("connections", "gdrive", "sign-out").returncode == 0, "sign-out failed")
    results.check(stage, "se connections gdrive: configure and status like the GUI (no browser)", google_drive)

    def integration():
        shown = se("integration", "context-menu", "--json")
        ensure(shown.returncode == 0, _text(shown))
        value = _json(shown)
        if not lib.WINDOWS:
            ensure(value == {"context_menu_supported": False, "context_menu": None}, value)
            ensure(se("integration", "context-menu", "on").returncode != 0, "Linux accepted the Explorer entry")
            return "Linux: not available, refused"
        ensure(value["context_menu_supported"] is True, value)
        return f"Windows: context menu {value['context_menu']}"
    results.check(stage, "se integration context-menu: Windows state, refused elsewhere", integration)

    def startup_check():
        # Without a feed the command ends with the missing-feed error, after it saved the choice.
        se("update", "--startup-check", "false", "--check")
        marker = client.app_data / "update_startup_check.txt"
        ensure(marker.is_file() and marker.read_text().strip() == "0", "the choice was not stored")
        se("update", "--startup-check", "true", "--check")
        ensure(not marker.exists(), "the default was not restored")
    results.check(stage, "se update --startup-check: the app's „Beim Start nach Updates suchen“", startup_check)
    lib.ai.stop_background_daemons([client])
