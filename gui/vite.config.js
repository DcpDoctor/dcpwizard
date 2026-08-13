import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  resolve: {
    // guikit sources sit outside gui/, so their bare imports miss gui/node_modules
    dedupe: ["@tauri-apps/api", "@tauri-apps/plugin-dialog"],
  },
  server: {
    port: 1421,
    strictPort: true,
    fs: {
      allow: [".."],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "esnext",
    minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_DEBUG,
  },
});
