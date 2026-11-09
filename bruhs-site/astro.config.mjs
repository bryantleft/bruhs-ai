import { defineConfig } from "astro/config";

// vercel.json's CSP allows only same-origin scripts and styles, so nothing may
// be inlined into the HTML. Astro inlines stylesheets under the asset limit
// ("auto") and scripts that import nothing and fit under Vite's
// assetsInlineLimit; both are off. test/build.test.ts fails if either returns.
export default defineConfig({
  site: "https://bruhs.ai",
  output: "static",
  trailingSlash: "never",
  build: { format: "file", inlineStylesheets: "never" },
  vite: { build: { assetsInlineLimit: 0 } },
  devToolbar: { enabled: false },
});
