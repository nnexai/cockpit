/// <reference types="vitest/config" />
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, statSync, utimesSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import { configDefaults } from "vitest/config";

type OutputSnapshot = Map<string, { digest: string; atimeMs: number; mtimeMs: number }>;

// Tauri embeds these files with include_bytes!, so Cargo tracks their mtimes.
function preserveUnchangedOutputTimes(): Plugin {
  let outDir = "";
  let before: OutputSnapshot = new Map();

  function files(dir: string): string[] {
    if (!existsSync(dir)) return [];
    return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) return files(path);
      return entry.isFile() ? [relative(outDir, path)] : [];
    });
  }

  return {
    name: "cockpit:preserve-unchanged-output-times",
    apply: "build",
    configResolved(config) {
      outDir = resolve(config.root, config.build.outDir);
    },
    buildStart() {
      before = new Map(files(outDir).map((file) => {
        const path = join(outDir, file);
        const { atimeMs, mtimeMs } = statSync(path);
        return [file, { digest: createHash("sha256").update(readFileSync(path)).digest("hex"), atimeMs, mtimeMs }];
      }));
    },
    writeBundle: {
      order: "post",
      handler() {
        const after = files(outDir);
        if (after.length !== before.size || after.some((file) => !before.has(file))) return;
        const unchanged = after.filter((file) =>
          createHash("sha256").update(readFileSync(join(outDir, file))).digest("hex") === before.get(file)!.digest
        );
        for (const file of unchanged) {
          const previous = before.get(file)!;
          utimesSync(join(outDir, file), previous.atimeMs / 1000, previous.mtimeMs / 1000);
        }
      },
    },
  };
}

export default defineConfig({
  root: ".",
  plugins: [react(), preserveUnchangedOutputTimes()],
  test: {
    exclude: [...configDefaults.exclude, ".stryker-tmp/**", "archive/**"],
  },
  build: {
    outDir: "dist",
  },
});
