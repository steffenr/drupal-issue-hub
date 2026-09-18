// Sync the app version across every file Tauri/npm read it from.
//
//   node scripts/sync-version.mjs <x.y.z>
//
// Sources updated:
//   - src-tauri/Cargo.toml        -> [package] version (the source of truth for
//                                    the binary + installer/bundle filenames)
//   - src-tauri/Cargo.lock        -> version of the "drupal-issue-hub" package entry
//   - src-tauri/tauri.conf.json   -> top-level "version"
//
// package.json / package-lock.json are intentionally NOT touched here: the
// release workflow bumps those via `npm version` so npm owns them.
import { readFileSync, writeFileSync } from 'node:fs';

const version = process.argv[2];
if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version || '')) {
  console.error('usage: node scripts/sync-version.mjs <x.y.z>');
  process.exit(1);
}

function updateFile(label, path, transform) {
  const before = readFileSync(path, 'utf8');
  const after = transform(before);
  if (after === null) {
    console.error(`sync-version: could not locate the version field in ${path}`);
    process.exit(1);
  }
  writeFileSync(path, after);
  console.log(`updated ${label} -> ${version}`);
}

// Cargo.toml: replace only the `version = "..."` inside the [package] block
// (leaves dependency `version = ...` lines such as tauri-build untouched).
updateFile('Cargo.toml', 'src-tauri/Cargo.toml', (text) => {
  const re = /^(\[package\][\s\S]*?^version = ")[^"]*(")/m;
  return re.test(text) ? text.replace(re, `$1${version}$2`) : null;
});

// Cargo.lock: set the version of the [[package]] block whose name is drupal-issue-hub.
updateFile('Cargo.lock', 'src-tauri/Cargo.lock', (text) => {
  const re = /(\[\[package\]\]\r?\nname = "drupal-issue-hub"\r?\nversion = ")[^"]*(")/;
  return re.test(text) ? text.replace(re, `$1${version}$2`) : null;
});

// tauri.conf.json: top-level version, preserving 2-space JSON formatting.
updateFile('tauri.conf.json', 'src-tauri/tauri.conf.json', (text) => {
  const conf = JSON.parse(text);
  conf.version = version;
  return JSON.stringify(conf, null, 2) + '\n';
});
