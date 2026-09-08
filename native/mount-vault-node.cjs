'use strict';

// Pinned Node/libuv application-pattern acceptance, not Obsidian certification.
// Contracts: nodejs/node v24.20.0 doc/api/fs.md (readdir, lstat, realpath, watch).
const assert = require('node:assert/strict');
const { watch } = require('node:fs');
const fs = require('node:fs/promises');
const path = require('node:path');
const { performance } = require('node:perf_hooks');

const BRANCHES = 512;
const LEVELS = 8;
const WIDE_FILES = 50001;
const root = process.argv[2];
const pending = { realpath: 0, readdir: 0, lstat: 0 };
const peak = { realpath: 0, readdir: 0, lstat: 0 };
const completed = { readdir: 0, lstat: 0, files: 0, directories: 0 };
let lastDirectory = performance.now();
let maxDirectoryGap = 0;
let firstFailure;
let watcher;
let watchEvents = 0;
let scanning = false;
const watchTarget = 'watch/node-startup-check.md';
let targetSeen = false;
let targetReady;
let saveArmed = false;
let unnamedSaveEvent = false;

function report(phase) {
  console.log(JSON.stringify({ phase, pending, peak, completed,
    max_directory_gap_ms: maxDirectoryGap, watch_events: watchEvents }));
}

function fatal(reason) {
  report('node-fatal');
  console.error(`[mount vault node] ${reason}`);
  // Rejection cannot cancel an outstanding filesystem operation. Parent owns
  // unmount-before-reaping on failure; the process never starts a second scan.
  process.exit(2);
}

const deadline = setTimeout(() => fatal('280-second overall deadline'), 280000);
const progress = setInterval(() => {
  if (scanning) {
    const gap = performance.now() - lastDirectory;
    maxDirectoryGap = Math.max(maxDirectoryGap, gap);
    if (gap >= 60000) fatal('60 seconds without a completed directory listing');
  }
  report('node-progress');
}, 5000);

async function operation(kind, work) {
  if (firstFailure) throw firstFailure;
  pending[kind]++;
  peak[kind] = Math.max(peak[kind], pending[kind]);
  try {
    const result = await work();
    if (kind === 'readdir') {
      const now = performance.now();
      maxDirectoryGap = Math.max(maxDirectoryGap, now - lastDirectory);
      lastDirectory = now;
      completed.readdir++;
    } else if (kind === 'lstat') {
      completed.lstat++;
    }
    return result;
  } catch (error) {
    firstFailure ??= error;
    throw error;
  } finally {
    pending[kind]--;
  }
}

function expected(relative) {
  if (relative === 'wide') return new Map(Array.from({ length: WIDE_FILES },
    (_, n) => [`f${String(n).padStart(5, '0')}.md`, false]));
  if (relative === 'large') return new Map(Array.from({ length: BRANCHES },
    (_, n) => [`b${String(n).padStart(3, '0')}`, true]));
  const parts = relative.split('/');
  assert.equal(parts[0], 'large');
  assert(/^b\d{3}$/.test(parts[1]));
  if (parts.length === 2) return new Map([['d0', true]]);
  const depth = parts.length - 3;
  assert(depth < LEVELS);
  for (let n = 0; n <= depth; n++) assert.equal(parts[n + 2], `d${n}`);
  const names = new Map(Array.from({ length: 4 }, (_, n) => [`note${n}.md`, false]));
  if (depth + 1 < LEVELS) names.set(`d${depth + 1}`, true);
  return names;
}

async function scan(relative) {
  const directory = path.join(root, ...relative.split('/'));
  const remaining = expected(relative);
  // Names, not Dirents: every name produces its own lstat before recursion.
  const names = await operation('readdir', () => fs.readdir(directory));
  assert.equal(names.length, remaining.size, `entry count: ${relative}`);
  const children = names.map(name => {
    assert.equal(typeof name, 'string');
    assert(remaining.has(name), `unexpected/duplicate: ${relative}/${name}`);
    const isDirectory = remaining.get(name);
    remaining.delete(name);
    return { name, isDirectory };
  });
  assert.equal(remaining.size, 0, `missing names: ${relative}`);
  // Match the observed child-promise fan-out, without raising libuv's pool or
  // substituting a fixture-specific worker cap. The finite manifest bounds work.
  const results = await Promise.allSettled(children.map(async ({ name, isDirectory }) => {
    const full = path.join(directory, name);
    const metadata = await operation('lstat', () => fs.lstat(full));
    assert.equal(metadata.isDirectory(), isDirectory, `kind: ${full}`);
    assert.equal(metadata.isSymbolicLink(), false, `unexpected link: ${full}`);
    if (isDirectory) {
      completed.directories++;
      await scan(`${relative}/${name}`);
    } else {
      assert(metadata.isFile(), `not a regular file: ${full}`);
      assert.equal(metadata.size, 4, `size: ${full}`);
      completed.files++;
    }
  }));
  for (const result of results) {
    if (result.status === 'rejected') throw result.reason;
  }
}

async function checkWatcherSave() {
  const full = path.join(root, ...watchTarget.split('/'));
  let created = false;
  let timer;
  try {
    saveArmed = true;
    const handle = await fs.open(full, 'wx');
    created = true;
    try { await handle.writeFile('node watcher save'); } finally { await handle.close(); }
    if (!targetSeen && !unnamedSaveEvent) {
      await new Promise((resolve, reject) => {
        targetReady = resolve;
        timer = setTimeout(() => reject(new Error('Node recursive watcher missed its controlled save')), 10000);
      });
    }
    if (!targetSeen) {
      // Node documents a nullable event filename, even on Windows. A nameless
      // event requires reconciliation, not a fabricated matching filename.
      assert(unnamedSaveEvent, 'no recursive Node notification during the save');
      assert((await fs.readdir(path.dirname(full))).includes(path.basename(full)));
      assert.equal((await fs.lstat(full)).size, Buffer.byteLength('node watcher save'));
    }
    console.log(JSON.stringify({ phase: 'node-watcher-save-delivered', watch_events: watchEvents,
      named_event: targetSeen, nameless_event_reconciled: !targetSeen }));
  } finally {
    clearTimeout(timer);
    targetReady = undefined;
    saveArmed = false;
    // CreateNew established ownership; never remove an unrelated existing file.
    if (created) await fs.unlink(full);
  }
}

async function main() {
  assert.equal(process.platform, 'win32');
  assert.equal(process.version, 'v24.20.0', 'suite Node version changed');
  assert.equal(process.versions.uv, '1.52.1', 'suite libuv version changed');
  assert(root && /^[A-Za-z]:\\$/.test(root), 'parent must supply the actual discovered drive root');
  console.log(JSON.stringify({ phase: 'node-start', node: process.version,
    libuv: process.versions.uv, root, uv_threadpool_size: process.env.UV_THREADPOOL_SIZE ?? 'default' }));
  const started = performance.now();
  const resolved = await operation('realpath', () => fs.realpath(root));
  assert.equal(path.resolve(resolved).toLowerCase(), path.resolve(root).toLowerCase());
  watcher = watch(resolved, { recursive: true }, (_event, filename) => {
    watchEvents++;
    if (typeof filename === 'string' && filename.replaceAll('\\', '/').toLowerCase() === watchTarget) {
      targetSeen = true;
      targetReady?.();
    } else if (filename == null && saveArmed) {
      unnamedSaveEvent = true;
      targetReady?.();
    }
  });
  watcher.on('error', error => fatal(`recursive watcher: ${error.code || error.message}`));
  console.log(JSON.stringify({ phase: 'node-watcher-active-before-scan', recursive: true }));
  scanning = true;
  lastDirectory = performance.now();
  const results = await Promise.allSettled(['large', 'wide'].map(scan));
  maxDirectoryGap = Math.max(maxDirectoryGap, performance.now() - lastDirectory);
  scanning = false;
  for (const result of results) if (result.status === 'rejected') throw result.reason;
  assert(maxDirectoryGap < 60000, 'directory-progress inactivity exceeded 60 seconds');
  assert.equal(completed.readdir, 4610);
  assert.equal(completed.directories, 4608);
  assert.equal(completed.files, 16384 + WIDE_FILES);
  assert.equal(completed.lstat, 4608 + 16384 + WIDE_FILES);
  assert.deepEqual(pending, { realpath: 0, readdir: 0, lstat: 0 });
  report('node-metadata-complete');
  console.log(JSON.stringify({ phase: 'node-elapsed', elapsed_ms: performance.now() - started,
    nested_directories: 4609, nested_files: 16384, wide_files: WIDE_FILES,
    content_read_calls: 0 }));
  await checkWatcherSave();
  report('node-complete');
}

main().catch(error => {
  report('node-error');
  console.error(error.stack || error);
  process.exitCode = 1;
}).finally(() => {
  watcher?.close();
  clearInterval(progress);
  clearTimeout(deadline);
});
