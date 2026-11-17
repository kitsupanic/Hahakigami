import { defineConfig, mergeConfig } from "vitest/config";
import viteConfig from "./vite.config";

export default mergeConfig(
  viteConfig,
  defineConfig({
    resolve: { conditions: ["browser"] },
    test: {
      include: ["src/**/*.test.ts"],
      setupFiles: ["src/testSetup.ts"],
    },
  }),
);
