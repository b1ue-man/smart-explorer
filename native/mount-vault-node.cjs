'use strict';

// One bounded Node workload, invoked only by the remote mount_vault_task suite.
// This covers this pinned Node/libuv combination, not an Obsidian certification.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const { performance } = require('node:perf_hooks');

const BRANCHES = 512;
const LEVELS = 8;
const WIDE_FILES = 50001;
const WORKERS = 16;
const root = process.argv[2];
const deadline = setTimeout(() => {
  console.error('[mount vault node] fatal 280-second metadata deadline');
  process.exit(2);
}, 280000);

async function checkDirectory(directory, expected, pointQueries) {
  const remaining = new Map(expected);
  const entries = await fs.readdir(directory, { withFileTypes: true });
  assert.equal(entries.length, remaining.size, `entry count: ${directory}`);
  for (const entry of entries) {
    assert(remaining.has(entry.name), `unexpected/duplicate: ${directory}/${entry.name}`);
    const isDirectory = remaining.get(entry.name);
    remaining.delete(entry.name);
    assert.equal(entry.isDirectory(), isDirectory, `dirent kind: ${entry.name}`);
    assert.equal(entry.isSymbolicLink(), false, `unexpected link: ${entry.name}`);
    if (!isDirectory) assert(entry.isFile(), `not a regular file: ${entry.name}`);
    if (pointQueries) {
      const full = path.join(directory, entry.name);
      for (const metadata of [await fs.stat(full), await fs.lstat(full)]) {
        assert.equal(metadata.isDirectory(), isDirectory, `stat kind: ${full}`);
        assert.equal(metadata.isSymbolicLink(), false, `stat link: ${full}`);
        if (!isDirectory) {
          assert(metadata.isFile(), `stat not regular: ${full}`);
          assert.equal(metadata.size, 4, `stat size: ${full}`);
        }
      }
    }
  }
  assert.equal(remaining.size, 0, `missing names: ${directory}`);
}

async function branch(number) {
  let directory = path.join(root, 'large', `b${String(number).padStart(3, '0')}`);
  await checkDirectory(directory, [['d0', true]], true);
  for (let depth = 0; depth < LEVELS; depth++) {
    directory = path.join(directory, `d${depth}`);
    const names = Array.from({ length: 4 }, (_, note) => [`note${note}.md`, false]);
    if (depth + 1 < LEVELS) names.push([`d${depth + 1}`, true]);
    await checkDirectory(directory, names, true);
  }
}

async function scanBranches() {
  let next = 0;
  let completed = 0;
  let failure;
  const workers = Array.from({ length: WORKERS }, async () => {
    while (!failure) {
      const number = next++;
      if (number >= BRANCHES) return;
      try {
        await branch(number);
        completed++;
      } catch (error) {
        failure ??= error;
      }
    }
  });
  // Finish every started async worker before reporting an error. The explicit
  // deadline plus parent unmount/child reaping covers an unresponsive driver.
  await Promise.allSettled(workers);
  if (failure) throw failure;
  assert.equal(completed, BRANCHES, 'recursive manifest incomplete');
}

async function main() {
  assert.equal(process.platform, 'win32');
  assert.equal(process.version, 'v24.20.0', 'suite Node version changed');
  assert.equal(process.versions.uv, '1.52.1', 'suite libuv version changed');
  assert(root && /^[A-Za-z]:\\$/.test(root), 'parent must supply the actual discovered drive root');
  console.log(JSON.stringify({ phase: 'node-start', node: process.version,
    libuv: process.versions.uv, root, workers: WORKERS }));
  const started = performance.now();
  await checkDirectory(path.join(root, 'large'), Array.from({ length: BRANCHES },
    (_, number) => [`b${String(number).padStart(3, '0')}`, true]), false);
  await scanBranches();
  await checkDirectory(path.join(root, 'wide'), Array.from({ length: WIDE_FILES },
    (_, number) => [`f${String(number).padStart(5, '0')}.md`, false]), false);
  // Metadata APIs on representative wide entries without turning the one
  // wide-enumeration contract into another 100,000-call point-stat campaign.
  for (const number of [0, 25000, 50000]) {
    const full = path.join(root, 'wide', `f${String(number).padStart(5, '0')}.md`);
    assert.equal((await fs.stat(full)).size, 4);
    assert.equal((await fs.lstat(full)).size, 4);
  }
  console.log(JSON.stringify({ phase: 'node-complete', elapsed_ms: performance.now() - started,
    nested_directories: 4609, nested_files: 16384, wide_files: WIDE_FILES, content_reads: 0 }));
}

main().catch(error => {
  console.error(error.stack || error);
  process.exitCode = 1;
}).finally(() => clearTimeout(deadline));
