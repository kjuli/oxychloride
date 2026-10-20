import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
    // The file icons are shared with the VS Code extension (../assets/file-icons).
    fs: { allow: [".", "../assets"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: "safari15",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    // A desktop app loads its bundle from disk; CodeMirror alone is a few hundred kB.
    chunkSizeWarningLimit: 1500,
  },
});
