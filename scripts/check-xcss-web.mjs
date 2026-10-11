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
assert.deepEqual(Object.keys(manifest.dependencies ?? {}).filter(name => name.startsWith("@xcss/")), ["@xcss/web"]);

for (const [name, version] of Object.entries({ react: "19.3.0", "react-dom": "19.3.0" })) {
  assert.equal(manifest.dependencies[name], version);
  assert.equal(lock.packages[`node_modules/${name}`].version, version);
  assert.match(lock.packages[`node_modules/${name}`].integrity, /^sha512-/u);
}
for (const [name, version] of Object.entries({ "@types/react": "19.3.0", "@types/react-dom": "19.3.0", "@vitejs/plugin-react": "6.1.2" })) {
  assert.equal(manifest.devDependencies[name], version);
  assert.equal(lock.packages[`node_modules/${name}`].version, version);
}
for (const packageName of ["@xcss/web"]) {
  const url = "https://github.com/isarmg/xcss/releases/download/v1.0.3/xcss-web-1.0.3.tgz";
  assert.equal(manifest.dependencies[packageName], url);
  const installed = JSON.parse(read(`node_modules/${packageName}/package.json`));
  assert.equal(installed.version, "1.0.3");
  assert.equal(lock.packages[`node_modules/${packageName}`].resolved, url);
  assert.match(lock.packages[`node_modules/${packageName}`].integrity, /^sha512-/u);
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
console.log("xcss 1.0.3 immutable React Web and complete font startup verified");

assert.equal(typeof prepareApplicationFonts, "function");
assert.equal(typeof startAfterFonts, "function");
