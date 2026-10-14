import { readFileSync, readdirSync } from "node:fs";
import { createXcssReactViteConfig } from "@xcss/web/web-toolchain/vite";
import { createHash } from "node:crypto";
import {fileURLToPath,pathToFileURL} from "node:url";
import {join} from "node:path";

const config = createXcssReactViteConfig({ base: "./" });
function moduleEntries(directory = "web/modules") {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = `${directory}/${entry.name}`;
    return entry.isDirectory() ? moduleEntries(path)
      : entry.name.endsWith(".ts") ? [[path.slice(4, -3), path]] : [];
  });
}
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
  ...config.build, outDir: "web/dist", assetsInlineLimit: 0, cssCodeSplit: false,
  rollupOptions: {
    input: {
      index: "web/index.ts", login: "web/login.ts",
      "dist/platform": "web/platform.ts",
      ...Object.fromEntries(moduleEntries()),
    }, preserveEntrySignatures: "strict",
    output: { format: "es", entryFileNames: "[name].js", chunkFileNames: "dist/[hash].js",
      assetFileNames: asset => asset.names.some(name => name.endsWith(".css")) ? "dist/platform.css" : "dist/[name][extname]" },
  },
};
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
