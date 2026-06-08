import { defineConfig } from "vite";
import { resolve } from "path";

// Tauri expects a fixed port; fail if it's not available.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  // prevent vite from obscuring rust errors
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    watch: {
      // tauri sources are watched by the rust side
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        settings: resolve(__dirname, "settings.html"),
        onboarding: resolve(__dirname, "onboarding.html"),
      },
    },
  },
});
