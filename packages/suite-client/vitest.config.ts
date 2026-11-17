import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["test/**/*.test.ts"],
    testTimeout: 20_000,
    // Le test d'intégration compile studiod si besoin : on lui laisse le temps.
    hookTimeout: 600_000,
  },
});
