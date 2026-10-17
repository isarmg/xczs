import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import {fileURLToPath,pathToFileURL} from "node:url";
import {join} from "node:path";
import { defineConfig } from "vite";
import { xcssWebStaticAssets } from "./web/xcss-web-assets.mjs";

const config = defineConfig({ base: "./", plugins: [] });
// A blocking same-origin stylesheet hides the body before first paint under CSP.
config.plugins.push({
  name: "xczs-font-startup-style",
  generateBundle() {
    this.emitFile({
      type: "asset", fileName: "dist/boot.css",
      source: readFileSync(new URL(import.meta.resolve("@xcss/web/web-fonts/boot.css"))),
    });
  },
});
config.build = {
  ...config.build, target: "esnext", modulePreload: { polyfill: false },
  outDir: "web/dist", assetsInlineLimit: 0, cssCodeSplit: false, sourcemap: false,
  rollupOptions: {
    input: {
      index: "web/xcss-web-index.js",
      login: "web/xcss-web-login.js",
      "dist/platform": "web/xcss-web-platform.js",
    }, preserveEntrySignatures: "strict",
    output: { format: "es", entryFileNames: "[name].js", chunkFileNames: "dist/[hash].js",
      assetFileNames: asset => asset.names.some(name => name.endsWith(".css")) ? "dist/platform.css" : "dist/[name][extname]" },
  },
};
config.plugins.push(xcssWebStaticAssets(), {
  name: "xczs-xcss-web-provenance-and-budget",
  generateBundle: { order: "post", handler(_options, bundle) {
    this.emitFile({ type: "asset", fileName: "assets/xcss-web-build.json", source: readFileSync(new URL("./web/pkg/xcss-web-build.json", import.meta.url)) });
    let total = 0;
    for (const [name, asset] of Object.entries(bundle)) {
      const size = asset.type === "chunk" ? Buffer.byteLength(asset.code) : typeof asset.source === "string" ? Buffer.byteLength(asset.source) : asset.source.byteLength;
      total += size;
      const limit = /\.woff2?$/iu.test(name) ? 16 * 1024 * 1024 : /\.wasm(?:\.gz)?$/u.test(name) ? 4 * 1024 * 1024 : 512 * 1024;
      if (name.endsWith(".map")) this.error("Frontend builds must not publish source maps");
      if (size > limit) this.error(`Asset ${name} is ${size} bytes; budget is ${limit}`);
    }
    if (total > 16 * 1024 * 1024) this.error("Frontend exceeds the 16 MiB asset budget");
  } },
});
// xcss's authored CSS masks are compile-time inputs, not user images.
// Emit them as immutable same-origin assets so Xczs keeps data: disallowed.
config.plugins.push({
  name: "xczs-same-origin-xcss-icons",
  enforce: "post",
  generateBundle: { order: "post", handler(_options, bundle) {
    for (const asset of Object.values(bundle)) {
      if (asset.type !== "asset" || !asset.fileName.endsWith(".css")) continue;
      asset.source = String(asset.source).replace(
        /url\("data:image\/svg\+xml,([^"\n]+)"\)/gu,
        (_match, encoded) => {
          const source = decodeURIComponent(encoded);
          const hash = createHash("sha256").update(source).digest("hex");
          const fileName = `dist/xcss-icon-${hash}.svg`;
          this.emitFile({ type: "asset", fileName, source });
          return `url("./${fileName.slice(5)}")`;
        },
      );
    }
  } },
});
export default async () => {
  if (process.env.XSSI_BUILD_TOOLS_DIR) {
    if (!process.env.XSSI_FRONTEND_METADATA_FILE) throw new Error("Explicit runtime inventory output is required");
    const {runtimeInventory} = await import(pathToFileURL(join(process.env.XSSI_BUILD_TOOLS_DIR,"scripts/frontend_build.mjs")));
    config.plugins.push(runtimeInventory({root:fileURLToPath(new URL("./",import.meta.url)),output:process.env.XSSI_FRONTEND_METADATA_FILE}));
  }
  return config;
};
