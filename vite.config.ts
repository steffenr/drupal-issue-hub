import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },

  build: {
    // The vendor chunk (TipTap/ProseMirror + React + marked + DOMPurify)
    // legitimately exceeds the default 500 kB threshold in a desktop app;
    // this is a code-splitting warning, not a correctness problem.
    chunkSizeWarningLimit: 800,
    // Split vendor libraries into their own chunk so the app code stays a
    // small, cache-friendly file. TipTap/ProseMirror loads once and is
    // separate from the app code that changes with every feature.
    rollupOptions: {
      output: {
        // Rolldown expects a function, not the object form.
        manualChunks(id) {
          const vendor = [
            "tiptap",
            "prosemirror",
            "markdown-it",
            "entities",
            "linkifyjs",
            "marked",
            "dompurify",
            "react",
            "react-dom",
            "scheduler",
          ];
          if (id.includes("node_modules")) {
            for (const pkg of vendor) {
              if (id.includes(`node_modules/${pkg}`)) {
                return "vendor";
              }
            }
          }
        },
      },
    },
  },
}));
