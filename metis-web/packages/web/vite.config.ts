import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { visualizer } from "rollup-plugin-visualizer";

export default defineConfig({
  plugins: [
    react(),
    visualizer({
      filename: "stats.html",
      template: "treemap",
    }),
  ],
  server: {
    port: 3000,
    proxy: {
      "/auth": "http://localhost:4000",
      "/api": "http://localhost:4000",
      "/health": "http://localhost:4000",
    },
  },
});
