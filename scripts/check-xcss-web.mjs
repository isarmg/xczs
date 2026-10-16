import {prepareApplicationFonts, startAfterFonts} from "@xcss/web/web-fonts";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const read = path => readFileSync(resolve(root, path));
const manifest = JSON.parse(read("package.json"));
const lock = JSON.parse(read("package-lock.json"));

for (const name of ["react", "react-dom"]) {
  assert.equal(manifest.dependencies[name], undefined);
  assert.equal(lock.packages[`node_modules/${name}`], undefined);
}
for (const name of ["@types/react", "@types/react-dom", "@vitejs/plugin-react"]) {
  assert.equal(manifest.devDependencies[name], undefined);
  assert.equal(lock.packages[`node_modules/${name}`], undefined);
}
for (const [name, entry] of Object.entries(lock.packages)) {
  if (!name) continue;
  assert.match(entry.resolved, /^https:\/\//u, name);
  assert.match(entry.integrity, /^sha512-/u, name);
}
assert.match(manifest.scripts["check:types"], /build-leptos\.py check/u);
assert.match(read("web/leptos/Cargo.toml").toString(), /leptos = .*csr/u);
for (const name of ["index", "login"]) {
  assert.match(read(`web/leptos-${name}.js`).toString(), /pkg\/xczs_web\.js/u);
}
const vite = read("vite.platform.config.mjs").toString();
assert.doesNotMatch(vite, /createXcssReactViteConfig|web\/index\.ts|web\/login\.ts/u);
assert.match(vite, /leptosStaticAssets/u);
for (const packageName of ["@xcss/web"]) {
  const url = "https://github.com/isarmg/xcss/releases/download/v1.0.0/xcss-web-1.0.0.tgz";
  assert.equal(manifest.dependencies[packageName], url);
  const installed = JSON.parse(read(`node_modules/${packageName}/package.json`));
  assert.equal(installed.version, "1.0.0");
  assert.equal(lock.packages[`node_modules/${packageName}`].resolved, url);
  assert.equal(lock.packages[`node_modules/${packageName}`].integrity, "sha512-QR/zmdzyQkLnFzTTp+EHYnOL9XEe5N95XVUasnDlKqdNWw/nVoY4evq083QTxMED/ElRRYeegEItnTxCUcb7TQ==");
}
const fonts = "node_modules/@xcss/web/dist/web-fonts";
const provenance = JSON.parse(read(`${fonts}/provenance.json`));
for (const [name, digest] of Object.entries(provenance.assets)) {
  const bytes = read(`${fonts}/${name}`);
  assert.equal(createHash("sha256").update(bytes).digest("hex"), digest, name);
}
assert.equal(provenance.latin.handwriting, false);
// Both Latin assets carry the same OFL text; the public OFL URL covers both.
assert.equal(provenance.assets["NORMAL-LICENSE.txt"], provenance.assets["OFL.txt"]);
for (const page of ["index.html", "login.html"]) {
  assert.match(read(`web/${page}`).toString(), /data-xcss-appearance="content-blocks"/u);
}
assert.match(read("xcss-product.toml").toString(), /web_profile = "web-react-admin"/u);
console.log("xcss 1.0.0 immutable static resources, Leptos runtime and complete font startup verified");

assert.equal(typeof prepareApplicationFonts, "function");
assert.equal(typeof startAfterFonts, "function");
