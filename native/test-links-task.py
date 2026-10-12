#!/usr/bin/env python3
"""One remote-only task suite for the links batch (2026-10-12).

Plan and expected results: docs/plaene/2026-10-12-links-in-se/plan.md (D).
Builds only the development `se` and the library test binary of this host (no
release build), then checks every milestone once:

  fmt     rustfmt of the batch's Rust files (stdin mode, no crate-wide debt)
  build   `se` plus the lib tests; compiler warnings inside new batch files fail
  unit    `links_task_` tests and the directly affected `cli::` tests
  local   a local tree with a file link, a directory link, a dangling link and a
          loop link (Linux: symlinks and a FIFO; Windows: junctions and, where
          the runner may create them, a file symlink): `se search`, `ls`,
          `stat`, `cat`, `cp -r`, `mv`, links named as source and destination
  mcp     the same tree through `se mcp`: search, list, read through a link
          (including a multi-page file), edit refusal, copy and move results
  sftp    (Linux) the tree on an OpenSSH server: search, stat, recursive copy,
          copy of a remote file link, refusal of a remote directory link,
          rename of a link and of a directory, move with kept directories
  ftp     (Linux) an FTP server without atomic no-replace rename: `se mv` of a
          file and of a directory falls back to copy and delete

Helpers (results, process runner, isolated clients, MCP client, OpenSSH
container) come from test-ai-plugin-task.py. Every stage runs even when an
earlier one failed; the summary lists them all.
"""
import argparse
import base64
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time

NATIVE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("ai_plugin_task", NATIVE / "test-ai-plugin-task.py")
ai = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ai)
# The shared fmt/build helpers read these module globals.
ai.BATCH_BASE = "1476fb9ec1828b2c66d95b1bf6d4cec13cca0595"
ai.NEW_FILE_PREFIXES = ("src/cli/entry_kind.rs", "src/cli/search.rs", "src/cli/link_paths.rs",
                        "src/cli/private_file.rs", "src/cli/links_task_", "src/cli/tree_plan.rs",
                        "src/cli/tree_remove.rs", "src/cli/tree_ops.rs", "src/cli/transfer.rs",
                        "src/cli/ops.rs", "src/cli/mcp/file_tools.rs", "src/cli/mcp/file_write.rs")
WINDOWS = ai.WINDOWS
ensure = ai.ensure
Client = ai.Client
McpSession = ai.McpSession


def stage_unit(results, log_root):
    completed = ai.run(["cargo", "test", "--locked", "--lib", "--", "links_task_", "cli::"],
                       cwd=ai.NATIVE, log=log_root / "unit.log", timeout=5400)
    output = (completed.stdout + completed.stderr).decode(errors="replace")
    summary = [line for line in output.splitlines() if line.startswith("test result:")]
    results.record("unit", "links_task_ + cli::", completed.returncode == 0,
                   output[-4000:] if completed.returncode else "; ".join(summary))
    ran = [line for line in output.splitlines() if line.startswith("test ") and "links_task_" in line]
    # Linux runs every links_task_ test; Windows the platform-neutral ones.
    results.record("unit", "links_task_ tests ran", len(ran) >= (3 if WINDOWS else 9), f"{len(ran)} tests")


# --------------------------------------------------------------------------- fixtures

def make_tree(root):
    """Returns {name: kind} of the entries a tree copy must skip."""
    (root / "sub").mkdir(parents=True)
    (root / "a.txt").write_text("alpha")
    (root / "sub" / "b.txt").write_text("beta")
    (root / "big.txt").write_text("".join(f"line {number:06d}\n" for number in range(40000)))
    skipped = {}
    if WINDOWS:
        for name, target in (("link_dir", root / "sub"), ("dangling", root / "missing"),
                             ("sub\\loop", root)):
            made = ai.run(["cmd", "/c", "mklink", "/J", str(root / name), str(target)])
            ensure(made.returncode == 0, made.stdout.decode(errors="replace") + made.stderr.decode(errors="replace"))
            skipped[name.replace("\\", "/")] = "l"
        try:
            os.symlink("a.txt", root / "link_file")
            os.symlink("big.txt", root / "link_big")
            skipped["link_file"] = skipped["link_big"] = "l"
        except OSError as error:
            print(f"[INFO] file symlinks unavailable on this runner ({error}); junction checks only",
                  flush=True)
    else:
        os.symlink("a.txt", root / "link_file")
        os.symlink("big.txt", root / "link_big")
        os.symlink("sub", root / "link_dir")
        os.symlink("missing", root / "dangling")
        os.symlink("..", root / "sub" / "loop")
        os.mkfifo(root / "fifo")
        skipped.update({"link_file": "l", "link_big": "l", "link_dir": "l", "dangling": "l",
                        "sub/loop": "l", "fifo": "s"})
    return skipped


def is_link(path):
    path = Path(path)
    return path.is_symlink() or (WINDOWS and path.is_junction())


def lines(completed):
    return completed.stdout.decode(errors="replace").splitlines()


def errors(completed):
    return completed.stderr.decode(errors="replace")


def hits(completed):
    """`se search` output as {relative path: kind marker}."""
    found = {}
    for line in lines(completed):
        marker, _size, rel = line.split("\t", 2)
        found[rel] = marker
    return found


# --------------------------------------------------------------------------- local

def stage_local(results, se_binary, work, log_root):
    stage = "local"
    client = Client(work, "local")
    log = log_root / "local.log"
    base = Path(tempfile.mkdtemp(prefix="se-links-local-", dir=work))
    tree = base / "tree"
    skipped = make_tree(tree)

    def se(*args):
        return client.se(se_binary, *args, log=log)

    def search():
        listed = se("search", str(tree), "", "--dirs")
        ensure(listed.returncode == 0, errors(listed))
        found = hits(listed)
        expected = {"a.txt": "-", "big.txt": "-", "sub": "d", "sub/b.txt": "-", **skipped}
        ensure(found == expected, f"{found} != {expected}")
        no_dirs = hits(se("search", str(tree), "b"))
        # The query matches entry names, not the path above them.
        expected_b = {"sub/b.txt": "-", "big.txt": "-",
                      **{name: "l" for name in skipped if "b" in name.rsplit("/", 1)[-1]}}
        ensure(no_dirs == expected_b, f"{no_dirs} != {expected_b}")
        named = hits(se("search", str(tree), "link*", "--glob"))
        ensure(set(named) == {name for name in skipped if name.startswith("link")}, named)
    results.check(stage, "search reports links and special files by name and never enters them", search)

    def help_text():
        text = se("search", "--help").stdout.decode()
        ensure("Also return matching directories" in text and "never entered" in text, text)
    results.check(stage, "search help: --dirs adds directories", help_text)

    def ls_stat():
        rows = {line.split("\t")[2]: line.split("\t")[0] for line in lines(se("ls", str(tree)))}
        ensure(rows["link_dir"] == "-l" and rows["sub"] == "d-" and rows["a.txt"] == "--", rows)
        stat = dict(line.split("\t", 1) for line in lines(se("stat", str(tree / "link_dir"))))
        ensure(stat["type"] == "symlink", stat)
        if not WINDOWS:
            ensure(rows["fifo"] == "s-", rows)
            fifo = dict(line.split("\t", 1) for line in lines(se("stat", str(tree / "fifo"))))
            ensure(fifo["type"] == "special", fifo)
    results.check(stage, "ls and stat name links and special files", ls_stat)

    def dangling():
        read = se("cat", str(tree / "dangling"))
        ensure(read.returncode != 0 and "is a link whose target" in errors(read), errors(read))
    results.check(stage, "cat of a dangling link names the link", dangling)

    def copy_tree():
        copied = se("cp", "-r", str(tree), str(base / "copy"))
        ensure(copied.returncode == 0, errors(copied))
        ensure((base / "copy" / "a.txt").read_text() == "alpha", "a.txt")
        ensure((base / "copy" / "sub" / "b.txt").read_text() == "beta", "sub/b.txt")
        for name in skipped:
            ensure(not os.path.lexists(base / "copy" / name), f"{name} was recreated")
        notices = [line for line in errors(copied).splitlines() if "skipped" in line]
        ensure(len(notices) == len(skipped), notices)
    results.check(stage, "cp -r copies the tree, skips and lists every link and special file", copy_tree)

    def named_links():
        into = base / "into"
        into.mkdir()
        real = base / "real"
        real.mkdir()
        via = base / "via"
        if WINDOWS:
            ensure(ai.run(["cmd", "/c", "mklink", "/J", str(via), str(real)]).returncode == 0, "junction via")
        else:
            os.symlink(real, via)
        directory = se("cp", "-r", str(tree / "link_dir"), str(into))
        ensure(directory.returncode == 0, errors(directory))
        ensure((into / "link_dir" / "b.txt").read_text() == "beta", "directory link copied as its tree")
        ensure(not os.path.lexists(into / "link_dir" / "loop"), "loop recreated")
        placed = se("cp", str(tree / "a.txt"), str(via))
        ensure(placed.returncode == 0 and (real / "a.txt").read_text() == "alpha", errors(placed))
        deeper = se("cp", str(tree / "a.txt"), str(via / "new" / "x.txt"))
        ensure(deeper.returncode == 0 and (real / "new" / "x.txt").read_text() == "alpha", errors(deeper))
        ensure(not is_link(real / "new"), "created folder is a link")
        if "link_file" in skipped:
            content = se("cp", str(tree / "link_file"), str(into))
            ensure(content.returncode == 0 and (into / "link_file").read_text() == "alpha", errors(content))
            ensure(not is_link(into / "link_file"), "copied as a link")
            refused = se("cp", "--force", str(tree / "sub" / "b.txt"), str(tree / "link_file"))
            ensure(refused.returncode != 0 and "link to a file" in errors(refused), errors(refused))
            ensure((tree / "a.txt").read_text() == "alpha", "written through the link")
    results.check(stage, "links named as source or destination are followed", named_links)

    def moves():
        moved_tree = base / "moved"
        source = base / "tree-for-move"
        make_tree(source)
        renamed = se("mv", str(source / "link_dir"), str(source / "link_renamed"))
        ensure(renamed.returncode == 0, errors(renamed))
        ensure(is_link(source / "link_renamed") and not os.path.lexists(source / "link_dir"), "link rename")
        whole = se("mv", "-r", str(source), str(moved_tree))
        ensure(whole.returncode == 0, errors(whole))
        ensure(not os.path.lexists(source), "source left behind")
        ensure(is_link(moved_tree / "link_renamed"), "link inside the renamed tree")
        ensure((moved_tree / "sub" / "b.txt").read_text() == "beta", "content")
    results.check(stage, "mv renames links and directories with links inside", moves)
    ai.stop_background_daemons([client])
    return tree, skipped


# --------------------------------------------------------------------------- MCP

def stage_mcp(results, se_binary, work, log_root, tree, skipped):
    stage = "mcp"
    client = Client(work, "mcp")
    logs = log_root / "mcp"
    logs.mkdir(parents=True, exist_ok=True)
    session = McpSession(se_binary, client, "initialize", logs, "links")
    try:
        def search():
            job = session.wait_job(session.call("search", {"target": str(tree), "directories": True}))
            ensure(job["status"] == "completed", job)
            ensure(job["hits"]["next_offset"] == job["hits_bytes"], "hits did not fit one page")
            found = {hit["relative"]: hit["type"] for hit in job["hits"]["items"]}
            kinds = {"l": "symlink", "s": "special"}
            for name, marker in skipped.items():
                ensure(found.get(name) == kinds[marker], f"{name}: {found.get(name)}")
            ensure(not any(rel.startswith(("sub/loop/", "link_dir/")) for rel in found), found)
        results.check(stage, "search hits carry symlink/special types", search)

        def listing():
            entries = {entry["name"]: entry["type"]
                       for entry in session.call("list", {"target": str(tree)})["entries"]}
            ensure(entries["link_dir"] == "symlink" and entries["sub"] == "dir", entries)
            if not WINDOWS:
                ensure(entries["fifo"] == "special", entries)
        results.check(stage, "list types", listing)

        if "link_big" in skipped:
            def read_through_link():
                small = session.call("read", {"target": str(tree / "link_file")})
                ensure(small["content"] == "alpha" and small["through_link"] is True, small)
                ensure(small["size"] is None and small["next_offset"] is None, small)
                expected = (tree / "big.txt").read_text()
                text, offset, pages = "", 0, 0
                while offset is not None:
                    page = session.call("read", {"target": str(tree / "link_big"), "offset": offset})
                    text += page["content"]
                    offset = page["next_offset"]
                    pages += 1
                    ensure(pages < 100, "read does not finish")
                ensure(pages > 1 and text == expected, f"{pages} pages, {len(text)} of {len(expected)} chars")
            results.check(stage, "read through a link pages the whole target file", read_through_link)

            def edit_link():
                refused = session.call("edit", {"target": str(tree / "link_file"), "edits": [
                    {"old_text": "alpha", "new_text": "omega"}]}, expect_error=True)
                ensure("is a link" in json.dumps(refused), refused)
                ensure((tree / "a.txt").read_text() == "alpha", "edited through the link")
            results.check(stage, "edit refuses a link before reading", edit_link)

        def copy_move():
            destination = tree.parent / "mcp-copy"
            copied = session.wait_job(session.call("copy", {"source": str(tree), "destination": str(destination),
                                                            "recursive": True}))
            ensure(copied["status"] == "completed", copied)
            omitted = {Path(item["path"]).as_posix().replace(tree.as_posix() + "/", "", 1): item["type"]
                       for item in copied["omitted"]}
            kinds = {"l": "symlink", "s": "special"}
            ensure(omitted == {name: kinds[marker] for name, marker in skipped.items()}, omitted)
            moved = session.wait_job(session.call("move", {"source": str(destination),
                                                           "destination": str(tree.parent / "mcp-moved"),
                                                           "recursive": True}))
            ensure(moved["status"] == "completed" and moved["omitted"] == [], moved)
            ensure(moved["kept_directories"] == [] and moved["destination"].endswith("mcp-moved"), moved)
        results.check(stage, "copy lists omitted entries, move renames", copy_move)
    finally:
        session.close()
        ai.stop_background_daemons([client])


# --------------------------------------------------------------------------- SFTP

REMOTE_TREE = ("mkdir -p {d}/sub && printf alpha > {d}/a.txt && printf beta > {d}/sub/b.txt && "
               "ln -s a.txt {d}/link_file && ln -s sub {d}/link_dir && ln -s missing {d}/dangling && "
               "ln -s .. {d}/sub/loop && mkfifo {d}/fifo")


def stage_sftp(results, se_binary, work, log_root):
    stage = "sftp"
    client = Client(work, "sftp")
    logs = log_root / "sftp"
    logs.mkdir(parents=True, exist_ok=True)
    try:
        container, port, user, password = ai.docker_ssh(work, log_root)
    except Exception as error:  # noqa: BLE001
        results.record(stage, "OpenSSH container fixture", False, error)
        return
    log = logs / "cli.log"

    def se(*args):
        return client.se(se_binary, *args, log=log, timeout=600)

    try:
        script = " && ".join(REMOTE_TREE.format(d=f"/config/{name}") for name in ("tree", "tree2", "tree3"))
        made = ai.run(["docker", "exec", "-u", f"{os.getuid()}:{os.getgid()}", container, "sh", "-c", script],
                      log=log)
        results.record(stage, "remote tree with links and a FIFO", made.returncode == 0, errors(made))
        added = client.se(se_binary, "connections", "add", "sftp", "--host", "127.0.0.1", "--port", str(port),
                          "--user", user, "--root", "/config", "--label", "box", "--password-stdin",
                          input_text=password + "\n", log=log)
        results.record(stage, "saved SFTP connection added", added.returncode == 0, errors(added))
        local = Path(tempfile.mkdtemp(prefix="se-links-sftp-", dir=work))

        def search_stat():
            listed = se("search", "@box:/tree", "", "--dirs")
            ensure(listed.returncode == 0, errors(listed))
            found = hits(listed)
            expected = {"a.txt": "-", "sub": "d", "sub/b.txt": "-", "link_file": "l", "link_dir": "l",
                        "dangling": "l", "sub/loop": "l", "fifo": "s"}
            ensure(found == expected, found)
            stat = dict(line.split("\t", 1) for line in lines(se("stat", "@box:/tree/link_file")))
            ensure(stat["type"] == "symlink", stat)
        results.check(stage, "search and stat over SFTP", search_stat)

        def copies():
            tree = se("cp", "-r", "@box:/tree", str(local))
            ensure(tree.returncode == 0, errors(tree))
            ensure((local / "tree" / "sub" / "b.txt").read_text() == "beta", "tree content")
            ensure(len([line for line in errors(tree).splitlines() if "skipped" in line]) == 5, errors(tree))
            content = se("cp", "@box:/tree/link_file", str(local / "via-link.txt"))
            ensure(content.returncode == 0 and (local / "via-link.txt").read_text() == "alpha", errors(content))
            refused = se("cp", "-r", "@box:/tree/link_dir", str(local / "dir-link"))
            ensure(refused.returncode != 0 and "link to a directory on a remote" in errors(refused),
                   errors(refused))
            ensure(not (local / "dir-link").exists(), "partial directory link copy")
        results.check(stage, "recursive copy, remote file link, remote directory link", copies)

        def renames():
            link = se("mv", "@box:/tree/link_file", "@box:/tree/link_moved")
            ensure(link.returncode == 0, errors(link))
            moved = dict(line.split("\t", 1) for line in lines(se("stat", "@box:/tree/link_moved")))
            ensure(moved["type"] == "symlink", moved)
            directory = se("mv", "-r", "@box:/tree3", "@box:/tree3-renamed")
            ensure(directory.returncode == 0, errors(directory))
            inside = dict(line.split("\t", 1) for line in lines(se("stat", "@box:/tree3-renamed/sub/loop")))
            ensure(inside["type"] == "symlink", inside)
            ensure(se("stat", "@box:/tree3").returncode != 0, "old directory still exists")
        results.check(stage, "mv renames a link and a directory on the server", renames)

        def move_out():
            moved = se("mv", "-r", "@box:/tree2", str(local / "moved"))
            ensure(moved.returncode == 0, errors(moved))
            ensure((local / "moved" / "sub" / "b.txt").read_text() == "beta", "moved content")
            kept = [line for line in errors(moved).splitlines() if "kept source directory" in line]
            ensure(len(kept) == 2, errors(moved))
            left = {line.split("\t")[2] for line in lines(se("ls", "@box:/tree2"))}
            ensure(left == {"link_file", "link_dir", "dangling", "fifo", "sub"}, left)
            ensure({line.split("\t")[2] for line in lines(se("ls", "@box:/tree2/sub"))} == {"loop"}, "sub")
        results.check(stage, "mv to another connection keeps links and their directories", move_out)

        def mcp_read():
            session = McpSession(se_binary, client, "modern", logs, "sftp")
            try:
                read = session.call("read", {"target": "@box:/tree/link_moved"})
                ensure(read["content"] == "alpha" and read["through_link"] is True, read)
            finally:
                session.close()
        results.check(stage, "MCP read through a remote link", mcp_read)
    finally:
        ai.run(["docker", "rm", "-f", container], timeout=120)
        ai.stop_background_daemons([client])


# --------------------------------------------------------------------------- FTP

def stage_ftp(results, se_binary, work, log_root):
    stage = "ftp"
    client = Client(work, "ftp")
    log = log_root / "ftp.log"
    served = Path(tempfile.mkdtemp(prefix="se-links-ftp-", dir=work))
    (served / "f.txt").write_text("file")
    (served / "d" / "inner").mkdir(parents=True)
    (served / "d" / "inner" / "x.txt").write_text("deep")
    password = base64.b16encode(os.urandom(8)).decode().lower()
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    installed = ai.run([sys.executable, "-m", "pip", "install", "pyftpdlib>=1.5.9,<3"], log=log, timeout=600)
    if installed.returncode != 0:
        results.record(stage, "pyftpdlib fixture", False, errors(installed))
        return
    server = subprocess.Popen([sys.executable, "-m", "pyftpdlib", "-i", "127.0.0.1", "-p", str(port), "-w",
                               "-d", str(served), "-u", "ftpuser", "-P", password],
                              stdout=open(log_root / "ftp-server.log", "wb"), stderr=subprocess.STDOUT)
    try:
        deadline = time.time() + 60
        while time.time() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=2) as connection:
                    if connection.recv(4).startswith(b"220"):
                        break
            except OSError:
                time.sleep(0.5)
        else:
            raise AssertionError("FTP server did not answer")

        def se(*args):
            return client.se(se_binary, *args, log=log, timeout=600)

        added = client.se(se_binary, "connections", "add", "ftp", "--host", "127.0.0.1", "--port", str(port),
                          "--user", "ftpuser", "--root", "/", "--label", "ftpbox", "--password-stdin",
                          input_text=password + "\n", log=log)
        results.record(stage, "saved FTP connection added", added.returncode == 0, errors(added))

        def move_file():
            moved = se("mv", "@ftpbox:/f.txt", "@ftpbox:/g.txt")
            ensure(moved.returncode == 0, errors(moved))
            ensure((served / "g.txt").read_text() == "file" and not (served / "f.txt").exists(), "file move")
        results.check(stage, "mv of a file without atomic rename copies and deletes", move_file)

        def move_directory():
            moved = se("mv", "-r", "@ftpbox:/d", "@ftpbox:/e")
            ensure(moved.returncode == 0, errors(moved))
            ensure((served / "e" / "inner" / "x.txt").read_text() == "deep", "directory content")
            ensure(not (served / "d").exists(), "source directory left")
        results.check(stage, "mv -r of a directory without atomic rename copies and deletes", move_directory)
    except Exception as error:  # noqa: BLE001
        results.record(stage, "FTP fixture", False, error)
    finally:
        server.terminate()
        try:
            server.wait(timeout=30)
        except subprocess.TimeoutExpired:
            server.kill()
        ai.stop_background_daemons([client])


# --------------------------------------------------------------------------- main

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--log-root", required=True)
    args = parser.parse_args()
    log_root = Path(args.log_root)
    log_root.mkdir(parents=True, exist_ok=True)
    results = ai.Results(log_root)
    ai.stage_fmt(results, log_root)
    built, _ = ai.stage_build(results, log_root)
    stage_unit(results, log_root)
    se_binary = ai.NATIVE / "target" / "debug" / ("se.exe" if WINDOWS else "se")
    if not built or not se_binary.exists():
        results.record("e2e", "development se binary", False, se_binary)
        return results.finish()
    work = Path(tempfile.mkdtemp(prefix="se-links-"))
    try:
        tree, skipped = stage_local(results, se_binary, work, log_root)
        stage_mcp(results, se_binary, work, log_root, tree, skipped)
        if not WINDOWS:
            stage_sftp(results, se_binary, work, log_root)
            stage_ftp(results, se_binary, work, log_root)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return results.finish()


if __name__ == "__main__":
    sys.exit(main())
