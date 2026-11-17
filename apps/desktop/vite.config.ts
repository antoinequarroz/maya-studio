import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Vite sert le front-end pour Tauri (`npm run tauri dev`) comme pour un navigateur
// (`npm run dev` avec VITE_STUDIO_URL / VITE_STUDIO_TOKEN, ou ?url=…&token=…).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: process.env.TAURI_DEV_HOST || "localhost",
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    // WebView2 (Windows) et WebKit récents (macOS 11+, Linux).
    target: "es2022",
    sourcemap: true,
  },
});
