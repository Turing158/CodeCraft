import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // Runtime captures can include third-party plugin trees. Never discover
    // or execute their tests as part of CodeCraft's application suite.
    include: ["src/**/*.{test,spec}.ts", "web/**/*.{test,spec}.ts"],
    maxWorkers: 2,
  },
});
