// Generates build/index.json — the manifest the desktop app reads to know which
// specs are available. Mirrors the layout of the upstream `specs.q.us-east-1`
// CDN's index.json so the spec:// protocol handler can point at our `build/`
// dir as a drop-in replacement.

import { readdir, writeFile, mkdir, copyFile } from "node:fs/promises";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const buildDir = join(root, "build");
const iconsSrc = join(root, "icons");
const iconsDest = join(buildDir, "icons");

async function copyIcons() {
  try {
    const files = await readdir(iconsSrc, { withFileTypes: true });
    await mkdir(iconsDest, { recursive: true });
    let n = 0;
    for (const f of files) {
      if (f.isFile() && f.name.endsWith(".png")) {
        await copyFile(join(iconsSrc, f.name), join(iconsDest, f.name));
        n += 1;
      }
    }
    console.log(`[autocomplete-specs] copied ${n} icons -> build/icons/`);
  } catch (err) {
    console.warn(`[autocomplete-specs] icons copy skipped: ${err.message}`);
  }
}

async function listSpecs() {
  const completions = [];
  const diffVersionedCompletions = [];

  let entries;
  try {
    entries = await readdir(buildDir, { withFileTypes: true });
  } catch {
    console.warn(
      "[autocomplete-specs] build/ does not exist yet. Run `pnpm build` first.",
    );
    return { completions, diffVersionedCompletions };
  }

  for (const entry of entries) {
    if (entry.name.startsWith(".") || entry.name.startsWith("@")) continue;
    if (entry.isDirectory()) {
      // Diff-versioned specs live in directories (e.g. `git/`, with index.js).
      diffVersionedCompletions.push(entry.name);
    } else if (entry.isFile() && entry.name.endsWith(".js")) {
      completions.push(entry.name.replace(/\.js$/, ""));
    }
  }
  completions.sort();
  diffVersionedCompletions.sort();
  return { completions, diffVersionedCompletions };
}

await mkdir(buildDir, { recursive: true });
await copyIcons();
const index = await listSpecs();
await writeFile(
  join(buildDir, "index.json"),
  JSON.stringify(index, null, 2) + "\n",
);
console.log(
  `[autocomplete-specs] wrote build/index.json (${index.completions.length} completions, ${index.diffVersionedCompletions.length} diff-versioned)`,
);
