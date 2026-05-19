import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath, URL } from "node:url";

const r = (p: string) => fileURLToPath(new URL(p, import.meta.url));

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [solid(), tailwindcss()],

  resolve: {
    alias: {
      "@": r("./src"),
    },
  },

  // Vite options tailored for Tauri
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    hmr: {
      protocol: "ws",
      host: "localhost",
      port: 1421,
    },
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: "es2022",
    minify: "esbuild",
    sourcemap: false,
    rollupOptions: {
      input: {
        settings: r("./index.html"),
        pill: r("./pill.html"),
        onboarding: r("./onboarding.html"),
        "tray-popover": r("./tray-popover.html"),
      },
    },
  },
});
