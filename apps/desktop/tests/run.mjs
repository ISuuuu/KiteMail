import { build } from "esbuild";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const outDir = path.join(root, "node_modules", ".tmp-test");
fs.mkdirSync(outDir, { recursive: true });
const outFile = path.join(outDir, "main-window.test.mjs");

await build({
  entryPoints: [path.join(root, "tests", "main-window.test.ts")],
  outfile: outFile,
  bundle: true,
  platform: "node",
  format: "esm",
  packages: "external",
});

const result = spawnSync(process.execPath, ["--test", outFile], {
  stdio: "inherit",
});
process.exit(result.status ?? 1);
