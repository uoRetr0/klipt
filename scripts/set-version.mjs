#!/usr/bin/env node
// Bumps Klipt's version in the four files that must agree, so releases don't
// drift. Run before tagging a release:
//
//   node scripts/set-version.mjs 0.3.3
//   npm run set-version -- 0.3.3      # (the -- is required by npm)
//
// Then commit, tag v<version>, and push the tag — the Release workflow builds
// the installers for every OS and drafts a GitHub release.
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const version = (process.argv[2] || "").trim().replace(/^v/, "");
if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error(`usage: node scripts/set-version.mjs <version>   e.g. 0.3.3`);
  console.error(version ? `  '${version}' is not a valid semver version` : "  (no version given)");
  process.exit(1);
}

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

// Each target is a single, anchored replacement — if the pattern ever stops
// matching (a file moved or reformatted), we fail loudly instead of silently
// leaving a version behind. Replacements are surgical to preserve formatting/EOL.
const targets = [
  ["package.json", /("version":\s*")[^"]+(")/],
  ["src-tauri/tauri.conf.json", /("version":\s*")[^"]+(")/],
  ["src-tauri/Cargo.toml", /(^version\s*=\s*")[^"]+(")/m],
  ["src-tauri/Cargo.lock", /(name = "klipt"\r?\nversion = ")[^"]+(")/],
];

let failed = false;
for (const [file, re] of targets) {
  const path = join(root, file);
  const src = readFileSync(path, "utf8");
  if (!re.test(src)) {
    console.error(`✗ version pattern not found in ${file}`);
    failed = true;
    continue;
  }
  writeFileSync(path, src.replace(re, (_m, a, b) => `${a}${version}${b}`));
  console.log(`✓ ${file} → ${version}`);
}
if (failed) process.exit(1);

console.log(`\nNext:`);
console.log(`  git commit -am "chore: bump version to ${version}"`);
console.log(`  git push origin main`);
console.log(`  git tag -a v${version} -m "Klipt ${version}" && git push origin v${version}`);
console.log(`\nThe Release workflow then builds every OS's installers into a draft release.`);
