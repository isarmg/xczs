import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { validateXcssWeb } from "../../../scripts/check-xcss-web.mjs";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const read = path => readFileSync(resolve(root, path));
const withFile = (path, contents) => name => name === path ? Buffer.from(contents) : read(name);

test("production Rust/WASM contract needs no installed npm package", () => {
  validateXcssWeb(path => {
    assert(!path.includes("node_modules"), path);
    return read(path);
  });
});

test("retired frontend dependencies cannot return through manifest or lock", () => {
  for (const name of ["react", "vite", "@vitejs/plugin-react", "@xcss/web"]) {
    const manifest = JSON.parse(read("package.json"));
    manifest.devDependencies[name] = "1.0.0";
    assert.throws(() => validateXcssWeb(withFile("package.json", JSON.stringify(manifest))), { code: "ERR_ASSERTION" });
    const lock = JSON.parse(read("package-lock.json"));
    lock.packages[`node_modules/${name}`] = {};
    assert.throws(() => validateXcssWeb(withFile("package-lock.json", JSON.stringify(lock))), { code: "ERR_ASSERTION" });
  }
});

test("both bootstrap entries must start the actual WASM application", () => {
  for (const entry of ["index", "login"]) {
    const path = `web/xcss-web-${entry}.js`;
    assert.throws(() => validateXcssWeb(withFile(path, read(path).toString().replace("xczs_web_bg.wasm", "missing.wasm"))), { code: "ERR_ASSERTION" });
  }
});

test("font bytes and immutable release attribution cannot drift", () => {
  const path = "web/vendor/xcss/web-fonts/MapleMonoFull-Regular.woff2";
  assert.throws(() => validateXcssWeb(withFile(path, "changed font")), { code: "ERR_ASSERTION" });
  const sourcePath = "web/vendor/xcss/SOURCE.json";
  const source = JSON.parse(read(sourcePath));
  source.npm.resolved = "https://github.com/isarmg/xcss/releases/download/v2.0.0/unpublished.tgz";
  assert.throws(() => validateXcssWeb(withFile(sourcePath, JSON.stringify(source))), { code: "ERR_ASSERTION" });
});

test("build commands must stay on the Rust builder", () => {
  const manifest = JSON.parse(read("package.json"));
  manifest.scripts["build:platform"] = "vite build";
  assert.throws(() => validateXcssWeb(withFile("package.json", JSON.stringify(manifest))), { code: "ERR_ASSERTION" });
});
