import { defineConfig } from "vite"
import vue from "@vitejs/plugin-vue"
import tailwindcss from "@tailwindcss/vite"

/** The `dimcode web` entry the dev server forwards /api and /ws to. */
const target = process.env.DIM_WEB_TARGET ?? "http://127.0.0.1:43201"
const unused = new URL("./src/vendor/unused-optional.ts", import.meta.url).pathname

export default defineConfig({
  base: "/",
  resolve: {
    alias: [
      // Curated shiki bundle on the JavaScript regex engine (see src/vendor/shiki-web.ts).
      { find: /^shiki$/, replacement: new URL("./src/vendor/shiki-web.ts", import.meta.url).pathname },
      { find: /^shiki\/wasm$/, replacement: unused },
      { find: /^@terrastruct\/d2$/, replacement: unused },
    ],
  },
  plugins: [tailwindcss(), vue()],
  server: {
    proxy: {
      "/api": { target, changeOrigin: true, headers: { origin: target } },
      "/ws": { target, changeOrigin: true, ws: true, headers: { origin: target } },
    },
  },
  build: {
    outDir: "dist",
    assetsInlineLimit: 0,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes("/node_modules/")) return undefined
          if (/\/node_modules\/(@vue|vue|vue-router|vue-i18n|@intlify|pinia)\//.test(id)) return "vendor-vue"
          return undefined
        },
      },
    },
  },
})
