import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig(({ command }) => ({
  // Electron loads production HTML through file://, so bundled assets must be
  // resolved relative to index.html rather than from the filesystem root.
  base: command === "build" ? "./" : "/",
  plugins: [react()],
  resolve: {
    // Glide is installed at the workspace root while the renderer resolves
    // React from the desktop package. Force every dependency to share the
    // renderer's React instance or hooks fail at runtime in development.
    dedupe: ["react", "react-dom"],
  },
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
}));
