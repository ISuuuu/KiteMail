import { build } from "esbuild";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const TEST_FILES = [
  "tests/main-window.test.ts",
  "tests/parity-shell.test.ts",
  "tests/parity-sidebar-list.test.ts",
  "tests/compose.test.ts",
  "tests/accounts.test.ts",
  "tests/reader-tools.test.ts",
  "tests/workspace-tools.test.ts",
  "tests/settings-shell.test.ts",
  "tests/parity-reader-drawer.test.ts",
  "tests/parity-shell-polish.test.ts",
  "tests/parity-embedded-views.test.ts",
  "tests/parity-pdf-thumb.test.ts",
  "tests/reader-startup.test.ts",
];

const outDir = path.resolve("node_modules/.tmp-test");
fs.mkdirSync(outDir, { recursive: true });

const builtFiles = [];
for (const entry of TEST_FILES) {
  const outFile = path.join(
    outDir,
    `${path.basename(entry, path.extname(entry))}.mjs`
  );
  await build({
    entryPoints: [entry],
    bundle: true,
    platform: "node",
    format: "esm",
    outfile: outFile,
    logLevel: "silent",
  });
  builtFiles.push(outFile);
}

execFileSync(process.execPath, ["--test", ...builtFiles], {
  stdio: "inherit",
});
