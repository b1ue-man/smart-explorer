// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Smart Explorer contributors
//
// se Desktop for GNOME Shell 45 to 48 (variant A). Starts one se Desktop
// surface per monitor (`<GUI> --desktop-surface --gnome-monitor <index>`) as
// a Meta.WaylandClient and keeps the windows of exactly these clients under
// all other windows: on every workspace, out of the window list, lowered
// again after every restacking, sized to the monitor. The surfaces report to
// the se Desktop shell over a Unix socket; this file only starts, watches and
// places them. The shell tells what to run in desktop-gnome.json and this file
// reports in desktop-extension.json (both in $XDG_RUNTIME_DIR/smart-explorer).
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const VARIANT = 'a';
const PROTOCOL = 1;
const EXIT_UPDATE_STOP = 75;
const EXIT_CHANNEL_END = 76;
const RESTART_DELAYS = [1, 5, 30];
const CRASH_WINDOW = 600;
const CRASH_LIMIT = 3;
const POLL_SECONDS = 2;

export default class SeDesktop extends Extension {
    enable() {
        const dir = GLib.build_filenamev([GLib.get_user_runtime_dir(), 'smart-explorer']);
        this._controlPath = GLib.build_filenamev([dir, 'desktop-gnome.json']);
        this._statePath = GLib.build_filenamev([dir, 'desktop-extension.json']);
        this._control = null;
        this._generation = null;
        this._entries = [];
        this._timers = new Set();
        this._crashes = [];
        this._halted = null;
        this._displayIds = [
            global.display.connect('window-created', (_display, window) => this._adopt(window)),
            global.display.connect('restacked', () => this._lowerAll()),
        ];
        try {
            this._monitor = Gio.File.new_for_path(this._controlPath)
                .monitor_file(Gio.FileMonitorFlags.NONE, null);
            this._monitorId = this._monitor.connect('changed', () => this._read());
        } catch (error) {
            console.log(`se-desktop: no file monitor: ${error.message}`);
        }
        this._poll = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, POLL_SECONDS,
            () => (this._read(), GLib.SOURCE_CONTINUE));
        this._read();
    }

    disable() {
        this._stopAll();
        for (const id of this._displayIds)
            global.display.disconnect(id);
        this._displayIds = [];
        if (this._monitor) {
            this._monitor.disconnect(this._monitorId);
            this._monitor.cancel();
            this._monitor = null;
        }
        if (this._poll)
            GLib.source_remove(this._poll);
        this._poll = 0;
        this._deleteState();
        this._control = null;
        this._generation = null;
    }

    _read() {
        let control = null;
        try {
            const [ok, bytes] = Gio.File.new_for_path(this._controlPath).load_contents(null);
            if (ok)
                control = JSON.parse(new TextDecoder().decode(bytes));
        } catch (_error) {
            control = null;
        }
        if (!control || control.protocol !== PROTOCOL || !Array.isArray(control.monitors)) {
            if (this._control) {
                this._stopAll();
                this._deleteState();
            }
            this._control = null;
            this._generation = null;
            return;
        }
        if (control.generation === this._generation)
            return;
        this._control = control;
        this._generation = control.generation;
        this._halted = null;
        this._crashes = [];
        this._stopAll();
        for (const item of control.monitors)
            this._spawn(item.index);
    }

    _geometry(index) {
        const item = this._control?.monitors.find(entry => entry.index === index);
        const shell = Main.layoutManager.monitors;
        const same = monitor => item && monitor.x === item.x && monitor.y === item.y &&
            monitor.width === item.width && monitor.height === item.height;
        // The file decides where a monitor lies (assumption: Mutter's order).
        return shell.find(same) ?? shell[index] ?? shell[0];
    }

    _spawn(index) {
        const argv = [this._control.exe, '--desktop-surface', '--gnome-monitor', String(index)];
        const launcher = new Gio.SubprocessLauncher({flags: Gio.SubprocessFlags.NONE});
        let client = null;
        let subprocess = null;
        try {
            try {
                client = Meta.WaylandClient.new(global.context, launcher);
            } catch (_error) {
                client = Meta.WaylandClient.new(launcher);
            }
            try {
                subprocess = client.spawnv(global.display, argv);
            } catch (_error) {
                subprocess = client.spawnv(argv);
            }
        } catch (error) {
            console.log(`se-desktop: cannot start the surface ${index}: ${error.message}`);
            this._failed(index);
            return;
        }
        const entry = {index, client, subprocess, pid: Number(subprocess.get_identifier()),
            handled: false, windows: []};
        this._entries.push(entry);
        global.get_window_actors().forEach(actor => this._adopt(actor.meta_window));
        subprocess.wait_async(null, (process, result) => {
            try {
                process.wait_finish(result);
            } catch (_error) {
                // The status below tells how it ended.
            }
            this._exited(entry, process);
        });
        this._writeState();
    }

    _adopt(window) {
        for (const entry of this._entries) {
            // Mutter 45 to 48: meta_wayland_client_owns_window.
            if (entry.windows.includes(window) || !entry.client.owns_window(window))
                continue;
            entry.handled = true;
            entry.windows.push(window);
            window.stick();
            entry.client.hide_from_window_list(window);
            window.lower();
            const place = this._geometry(entry.index);
            window.move_resize_frame(false, place.x, place.y, place.width, place.height);
            window.connect('unmanaged', () => {
                entry.windows = entry.windows.filter(known => known !== window);
                this._writeState();
            });
            this._writeState();
        }
    }

    _lowerAll() {
        for (const entry of this._entries)
            entry.windows.forEach(window => window.lower());
    }

    _exited(entry, process) {
        if (!this._entries.includes(entry))
            return;
        this._entries = this._entries.filter(known => known !== entry);
        const code = process.get_if_exited() ? process.get_exit_status() : -1;
        this._writeState();
        // 75 and 76 stay off until the next generation; 0 (the shell stopped
        // it) starts again without counting as a crash.
        if (code === EXIT_UPDATE_STOP || code === EXIT_CHANNEL_END)
            return;
        if (code === 0) {
            this._later(1, entry.index);
            return;
        }
        this._failed(entry.index);
    }

    _failed(index) {
        const now = GLib.get_real_time() / 1e6;
        this._crashes = this._crashes.filter(at => now - at < CRASH_WINDOW);
        this._crashes.push(now);
        if (this._crashes.length >= CRASH_LIMIT) {
            this._halted = {reason: 'crash_loop', at: Math.floor(now)};
            this._writeState();
            return;
        }
        this._later(RESTART_DELAYS[Math.min(this._crashes.length, RESTART_DELAYS.length) - 1], index);
    }

    _later(seconds, index) {
        const id = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, seconds, () => {
            this._timers.delete(id);
            if (this._control && !this._halted)
                this._spawn(index);
            return GLib.SOURCE_REMOVE;
        });
        this._timers.add(id);
    }

    _stopAll() {
        for (const id of this._timers)
            GLib.source_remove(id);
        this._timers.clear();
        const stopped = this._entries;
        this._entries = [];
        for (const entry of stopped) {
            try {
                entry.subprocess.send_signal(15);
            } catch (_error) {
                // Already gone.
            }
        }
        this._writeState();
    }

    _writeState() {
        const state = {
            version: 1,
            variant: VARIANT,
            surfaces: this._entries.map(entry =>
                ({monitor: entry.index, pid: entry.pid, handled: entry.handled})),
            halted: this._halted,
        };
        try {
            Gio.File.new_for_path(this._statePath).replace_contents(
                new TextEncoder().encode(JSON.stringify(state)), null, false,
                Gio.FileCreateFlags.PRIVATE | Gio.FileCreateFlags.REPLACE_DESTINATION, null);
        } catch (_error) {
            // The shell's folder is gone.
        }
    }

    _deleteState() {
        try {
            Gio.File.new_for_path(this._statePath).delete(null);
        } catch (_error) {
            // Not there.
        }
    }
}
