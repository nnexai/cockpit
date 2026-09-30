import { lstatSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { mergeConfig, type ResolvedConfig } from "vite";
import normalConfig from "../vite.config";

// Stryker 10's tsconfig rewriter calls the removed TypeScript 5 JS API.
// Restore the exact config after preprocessing, before Vite transforms sources.
export default mergeConfig(normalConfig, {
  plugins: [
    {
      name: "cockpit-mutation-tsconfig",
      configResolved(config: ResolvedConfig) {
        const checkout = process.env.COCKPIT_MUTATION_ROOT;
        const original = process.env.COCKPIT_MUTATION_TSCONFIG;
        if (!checkout || !original) {
          throw new Error("Mutation Vite config requires the quality:mutation launcher");
        }
        const sandboxParent = join(realpathSync(checkout), ".stryker-tmp");
        const sandbox = realpathSync(config.root);
        if (realpathSync(sandboxParent) !== sandboxParent || dirname(sandbox) !== sandboxParent) {
          throw new Error("Refusing to restore tsconfig outside a Stryker sandbox");
        }
        const target = join(sandbox, "tsconfig.json");
        try {
          writeFileSync(target, original, { encoding: "utf8", flag: "wx" });
        } catch (error) {
          if (!(error instanceof Error && "code" in error && error.code === "EEXIST")) {
            throw error;
          }
        }
        if (!lstatSync(target).isFile() || readFileSync(target, "utf8") !== original) {
          throw new Error("Mutation sandbox contains an unexpected tsconfig");
        }
      },
    },
  ],
});
