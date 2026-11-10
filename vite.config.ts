import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));

// Two pages (widget, settings) served by the Tauri windows. The dev server
// port is fixed because src-tauri/tauri.conf.json points at it.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  publicDir: here("./src/public"),
  resolve: {
    alias: { "@": here("./src") },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    rollupOptions: {
      input: {
        widget: here("./widget.html"),
        settings: here("./settings.html"),
      },
    },
  },
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts", "src/**/*.test.ts"],
    restoreMocks: true,
    unstubGlobals: true,
  },
});
