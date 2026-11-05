import { defineConfig } from "vite"

export default defineConfig({
  define: { "process.env.NODE_ENV": '"production"', __VUE_OPTIONS_API__: false, __VUE_PROD_DEVTOOLS__: false, __VUE_PROD_HYDRATION_MISMATCH_DETAILS__: false },
  build: { lib: { entry: "src/panel.js", formats: ["es"], fileName: () => "panel.js" } },
})
