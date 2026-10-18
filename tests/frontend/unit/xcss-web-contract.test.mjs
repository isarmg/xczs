import assert from "node:assert/strict";
import { createHash } from "node:crypto";
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

test("font bytes cannot drift from the reviewed Git asset snapshot", () => {
  const path = "web/vendor/xcss/web-fonts/MapleMonoFull-Regular.woff2";
  assert.throws(() => validateXcssWeb(withFile(path, "changed font")), { code: "ERR_ASSERTION" });
});

test("Git source identity rejects dirty, altered and legacy package attribution", () => {
  const sourcePath = "web/vendor/xcss/SOURCE.json";
  const mutate = [
    source => { source.base_revision += "-dirty"; },
    source => { source.base_revision = "f".repeat(40); },
    source => { source.origin = "https://github.com/another-owner/xcss"; },
    source => { source.source_directory = "unreviewed-assets"; },
    source => { source.package = "@another-owner/web"; },
    source => { source.version = "2.0.1"; },
    source => { source.format = 2; },
    source => { source.dirty = true; },
    source => { source.npm = { resolved: "https://github.com/isarmg/xcss/releases/download/v1.0.0/xcss-web-1.0.0.tgz" }; },
  ];
  for (const change of mutate) {
    const source = JSON.parse(read(sourcePath));
    change(source);
    assert.throws(() => validateXcssWeb(withFile(sourcePath, JSON.stringify(source))), {
      code: "ERR_ASSERTION", message: /reviewed XCSS asset receipt/u,
    });
  }
});

test("editing an asset together with its declared digest cannot forge the source receipt", () => {
  const sourcePath = "web/vendor/xcss/SOURCE.json";
  const asset = "admin-ui/styles.css";
  const replacement = Buffer.from(".forged-style { color: red; }\n");
  const source = JSON.parse(read(sourcePath));
  source.files[asset] = createHash("sha256").update(replacement).digest("hex");
  assert.throws(() => validateXcssWeb(path => {
    if (path === sourcePath) return Buffer.from(JSON.stringify(source));
    if (path === `web/vendor/xcss/${asset}`) return replacement;
    return read(path);
  }), { code: "ERR_ASSERTION", message: /reviewed XCSS asset receipt/u });
});

test("the reviewed asset inventory and recorded digests cannot be replaced or omitted", () => {
  const sourcePath = "web/vendor/xcss/SOURCE.json";
  for (const change of [
    source => { source.files["admin-ui/styles.css"] = "0".repeat(64); },
    source => { delete source.files["LICENSE"]; },
    source => { source.files["admin-ui/server-corners.js"] = "0".repeat(64); },
  ]) {
    const source = JSON.parse(read(sourcePath));
    change(source);
    assert.throws(() => validateXcssWeb(withFile(sourcePath, JSON.stringify(source))), {
      code: "ERR_ASSERTION", message: /reviewed XCSS asset receipt/u,
    });
  }
});

test("build commands must stay on the Rust builder", () => {
  const manifest = JSON.parse(read("package.json"));
  manifest.scripts["build:platform"] = "vite build";
  assert.throws(() => validateXcssWeb(withFile("package.json", JSON.stringify(manifest))), { code: "ERR_ASSERTION" });
});
