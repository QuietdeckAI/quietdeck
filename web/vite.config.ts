import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// `pnpm dev` proxies the API to a local deck (QUIETDECK_DEV_DECK, default http://localhost:8080).
const deck = process.env.QUIETDECK_DEV_DECK ?? "http://localhost:8080";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(import.meta.dirname, "src") } },
  server: { proxy: { "/api": { target: deck, changeOrigin: false } } },
  build: { outDir: "dist", emptyOutDir: true, sourcemap: false },
});
