"""Share and Exec settings on every surface (GUI parity batch, 2026-10-16).

Called only by native/test-luecken-task.py (`lib` is that module); no entry
point of its own. Plan: docs/plaene/2026-10-16-luecken-schliessen/plan.md (I),
surface inventory: docs/oberflaechen/share-exec.md.

stage_surfaces  (Linux and Windows) the CLI commands added for parity with
                the desktop and Android GUI, in an isolated client without a
                paired device: device name, Direct code renewal, request
                policy, room create/list/rename/leave/rejoin/code/remove,
                export order, device list, host trash (Windows lists it,
                other hosts say it is Windows-only), grants listing shape
paired_checks   (Linux, inside the `exec` stage with its paired clients) the
                Exec account and program list in `se share grants` (text and
                JSON) and `se share status --json` and the device settings of
                a real contact (`se share device list/set`);
direct_online_last  (Linux, last check of that pair) the Direct code
                reachability switch of a running worker
The GUIs themselves are covered by the `unit` stage (desktop page model, exec
program rules, bridge methods) and the `android`/`android-device` jobs (JVM
tests, `parity-check` phase of LueckenUpgradeTaskTest).
"""
import json
import tempfile
from pathlib import Path


def _json(completed):
    return json.loads(completed.stdout.decode(errors="replace"))


def _text(completed):
    return (completed.stdout + b"\n" + completed.stderr).decode(errors="replace")


def stage_surfaces(lib, results, se_binary, work, log_root):
    stage = "surfaces"
    logs = log_root / "surfaces"
    logs.mkdir(parents=True, exist_ok=True)
    log = logs / "surfaces.log"
    client = lib.Client(work, "surfaces")
    ensure = lib.ensure

    def se(*args, input_text=None):
        return client.se(se_binary, *args, input_text=input_text, log=log)

    def identity_settings():
        renamed = se("share", "identity", "--set-name", "Luecken Parity", "--json")
        ensure(renamed.returncode == 0, _text(renamed))
        ensure(_json(renamed)["device_name"] == "Luecken Parity", _text(renamed))
        before = _json(se("share", "identity", "--json"))["direct_code"]
        refused = se("share", "identity", "--rotate-code", input_text="no\n")
        ensure(refused.returncode != 0, "the code was renewed without YES")
        ensure(_json(se("share", "identity", "--json"))["direct_code"] == before, "a refused renewal changed the code")
        rotated = se("share", "identity", "--rotate-code", "--yes", "--json")
        ensure(rotated.returncode == 0, _text(rotated))
        after = _json(rotated)["direct_code"]
        ensure(after != before and after == _json(se("share", "identity", "--json"))["direct_code"], (before, after))
        conflict = se("share", "identity", "--set-name", "x", "--rotate-code", "--yes")
        ensure(conflict.returncode != 0, "two identity changes in one call were accepted")
    results.check(stage, "se share identity: --set-name, --rotate-code (asks YES, else unchanged)", identity_settings)

    def request_policy():
        ensure("request_policy\task" in _text(se("share", "request", "policy")), "default is not ask")
        auto = se("share", "request", "policy", "auto-accept", "--json")
        ensure(auto.returncode == 0 and _json(auto)["request_policy"] == "auto-accept", _text(auto))
        listed = _json(se("share", "export", "list", "--json"))
        ensure(listed["request_policy"] == "AutoAccept", listed.get("request_policy"))
        ask = se("share", "request", "policy", "ask")
        ensure(ask.returncode == 0 and "request_policy\task" in _text(ask), _text(ask))
        ensure(se("share", "request", "policy", "always").returncode != 0, "an unknown policy was accepted")
    results.check(stage, "se share request policy: ask/auto-accept stored in the file the GUIs read", request_policy)

    def rooms():
        created = se("share", "room", "create", "--name", "Luecken")
        ensure(created.returncode == 0, _text(created))
        listed = _json(se("share", "room", "list", "--json"))
        ensure(listed["count"] == 1 and listed["rooms"][0]["name"] == "Luecken", listed)
        room = listed["rooms"][0]
        ensure(room["auto_join"] is True and room["policy"]["members_may_write"] is False, room)
        renamed = se("share", "room", "rename", room["id"], "  Luecken-Werkstatt ")
        ensure(renamed.returncode == 0 and "changed=true" in _text(renamed), _text(renamed))
        left = se("share", "room", "leave", "Luecken-Werkstatt")
        ensure(left.returncode == 0, _text(left))
        state = _json(se("share", "room", "list", "--json"))["rooms"][0]
        ensure(state["name"] == "Luecken-Werkstatt" and state["auto_join"] is False, state)
        ensure("joined=false" in _text(se("share", "room", "list")), "text listing lacks the joined state")
        ensure(se("share", "room", "rejoin", room["room_id"]).returncode == 0, "rejoin failed")
        ensure(_json(se("share", "room", "list", "--json"))["rooms"][0]["auto_join"] is True, "rejoin was not stored")
        code = _json(se("share", "room", "code", room["id"], "--json"))["code"]
        ensure(code.startswith("SE-R"), code)
        missing = se("share", "room", "member", room["id"], "nobody", "admit")
        ensure(missing.returncode != 0 and "member not found" in _text(missing), _text(missing))
        removed = se("share", "room", "remove", room["id"])
        ensure(removed.returncode == 0, _text(removed))
        ensure(_json(se("share", "room", "list", "--json"))["count"] == 0, "the room was not removed")
    results.check(stage, "se share room: create, list (JSON/text), rename, leave, rejoin, code, member check, remove",
                  rooms)

    def export_order():
        base = Path(tempfile.mkdtemp(prefix="luecken-exports-", dir=work))
        first, second = base / "first", base / "second"
        first.mkdir()
        second.mkdir()
        for path, label in ((first, "LueckenFirst"), (second, "LueckenSecond")):
            added = se("share", "export", "add", str(path), "--label", label)
            ensure(added.returncode == 0, _text(added))

        def labels():
            roots = _json(se("share", "export", "list", "--json"))["roots"]
            return [root["label"] for root in roots if root["label"].startswith("Luecken")]
        ensure(labels() == ["LueckenFirst", "LueckenSecond"], labels())
        moved = se("share", "export", "move", "LueckenSecond", "--up")
        ensure(moved.returncode == 0 and "position=" in _text(moved), _text(moved))
        ensure(labels() == ["LueckenSecond", "LueckenFirst"], labels())
        for _ in range(8):  # up to the top; a client may hold other exports first
            top = se("share", "export", "move", "LueckenSecond", "--up")
            ensure(top.returncode == 0, _text(top))
            if "moved=false" in _text(top):
                break
        else:
            raise AssertionError("the export never reached the top")
        ensure(_json(se("share", "export", "list", "--json"))["roots"][0]["label"] == "LueckenSecond",
               "the top export is another one")
        ensure(se("share", "export", "move", "LueckenSecond").returncode != 0, "a move without direction was accepted")
        for label in ("LueckenFirst", "LueckenSecond"):
            ensure(se("share", "export", "remove", label).returncode == 0, f"remove {label}")
    results.check(stage, "se share export move: the order peers see, like the GUI's up/down", export_order)

    def devices_and_grants():
        devices = _json(se("share", "device", "list", "--json"))
        ensure(devices["count"] == 0 and devices["devices"] == [], devices)
        ensure(se("share", "device", "set", "--auto-open", "true").returncode != 0, "set without a device succeeded")
        ensure(se("share", "device", "set", "x").returncode != 0, "set without a setting succeeded")
        grants = _json(se("share", "grants", "--json"))
        ensure(grants["count"] == 0, grants)
    results.check(stage, "se share device / grants on a client without devices: empty lists, clear refusals",
                  devices_and_grants)

    def host_trash():
        listed = se("share", "trash", "list", "--json")
        if lib.WINDOWS:
            ensure(listed.returncode == 0 and "entries" in _json(listed), _text(listed))
            return "Windows host trash listed"
        ensure(listed.returncode != 0 and "Windows" in _text(listed), _text(listed))
        return "refused outside Windows with the reason"
    results.check(stage, "se share trash: the desktop tab „Papierkorb“ on the terminal", host_trash)
    lib.ai.stop_background_daemons([client])


def paired_checks(lib, results, se_binary, a, b, log):
    """Inside the exec stage: `b` hosts the grant of `a` with the program list /usr/bin/id."""
    stage = "exec"
    ensure = lib.ensure

    def grants_listing():
        listed = _json(b.se(se_binary, "share", "grants", "--json", log=log))
        exec_block = listed["grants"][0]["exec"]
        ensure(exec_block["run_as"] == "standard" and exec_block["programs"] == ["/usr/bin/id"], exec_block)
        ensure(exec_block["enabled"] is True and "policy_revision" in exec_block, exec_block)
        line = _text(b.se(se_binary, "share", "grants", log=log))
        ensure("\texec_run_as=standard\texec_programs=/usr/bin/id" in line, line)
        status = _json(b.se(se_binary, "share", "status", "--json", log=log))
        ensure(status["grants"][0]["exec"]["programs"] == ["/usr/bin/id"], status["grants"][0])
    results.check(stage, "se share grants/status (text and JSON) name the Exec account and program list",
                  grants_listing)

    def device_settings():
        devices = _json(a.se(se_binary, "share", "device", "list", "--json", log=log))["devices"]
        ensure(len(devices) == 1, devices)
        device = devices[0]
        ensure(device["auto_connect"] is True and device["auto_open"] is False and device["access"], device)
        changed = a.se(se_binary, "share", "device", "set", device["id"], "--auto-open", "true", log=log)
        ensure(changed.returncode == 0 and "changed=true" in _text(changed), _text(changed))
        ensure(_json(a.se(se_binary, "share", "device", "list", "--json", log=log))["devices"][0]["auto_open"] is True,
               "auto_open was not stored")
        back = a.se(se_binary, "share", "device", "set", device["id"], "--auto-open", "false", log=log)
        ensure(back.returncode == 0, _text(back))
    results.check(stage, "se share device list/set on a paired client (automatically open)", device_settings)



def direct_online_last(lib, results, se_binary, b, log):
    """Last check of the exec pair: switching the direct code off may end its sessions."""
    ensure = lib.ensure

    def direct_online():
        for value in ("false", "true"):
            switched = b.se(se_binary, "share", "identity", "--direct-online", value, log=log)
            ensure(switched.returncode == 0 and f"direct_online\t{value}" in _text(switched), _text(switched))
    results.check("exec", "se share identity --direct-online on a running worker", direct_online)
