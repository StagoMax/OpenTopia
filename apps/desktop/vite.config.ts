import { defineConfig } from "vite"
import react from "@vitejs/plugin-react"

export default defineConfig(({ command }) => ({
  // Electron loads production HTML through file://, so bundled assets must be
  // resolved relative to index.html rather than from the filesystem root.
  base: command === "build" ? "./" : "/",
  plugins: [react()],
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
}))
