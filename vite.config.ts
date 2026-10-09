/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { configDefaults } from "vitest/config";

export default defineConfig({
  root: ".",
  plugins: [react()],
  test: {
    exclude: [...configDefaults.exclude, ".stryker-tmp/**", "archive/**"],
  },
  build: {
    outDir: "dist",
  },
});
