// Vite Dev Server Configuration
//
//   React plugin, dev port 5173, and proxy /api to gateway at localhost:7700.
//
// INPUT:
//   - vite.config.ts (this file)
//
// OUTPUT:
//   - Dev server with API proxy stripping /api prefix
//
// NOTES:
//   Production builds have no proxy; deploy gateway with CORS or reverse proxy.
//
// Written by Composer for Ze Supply Chain. June 2025. All rights reserved.

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": {
        target: "http://127.0.0.1:7700",
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/api/, ""),
      },
    },
  },
});
