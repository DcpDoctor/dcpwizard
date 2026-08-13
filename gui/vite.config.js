import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

// guikit sources sit outside gui/, so their bare imports miss gui/node_modules
const scopedPackages = fileURLToPath(new URL("node_modules/@tauri-apps/", import.meta.url));

export default defineConfig({
  clearScreen: false,
  resolve: {
    alias: [{ find: /^@tauri-apps\//, replacement: scopedPackages }],
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
