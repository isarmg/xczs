import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
// Review the committed source identity and its complete file receipt together.
// This also prevents changing an asset and simply replacing its recorded hash.
const xcssAssetsRevision = "a055fdf5ce4bdf6cb943284813e42ba864c74b9d";
const xcssAssetsReceiptSha256 = "3bccf5dad68ce08dff8f3b0b318b3d81a3aee294eb9575c62de42e06692ccfb6";

// This check must also work before npm ci. The deployed bytes come from the
// reviewed vendor snapshot, not the historical npm package's JS API.
export function validateXcssWeb(read = path => readFileSync(resolve(root, path))) {
  const manifest = JSON.parse(read("package.json"));
  const lock = JSON.parse(read("package-lock.json"));
  const retired = ["react", "react-dom", "@types/react", "@types/react-dom", "@vitejs/plugin-react", "vite", "@xcss/web"];
  for (const name of retired) {
    assert.equal(manifest.devDependencies[name], undefined, name);
    assert.equal(manifest.dependencies?.[name], undefined, name);
    assert.equal(lock.packages[`node_modules/${name}`], undefined, name);
    assert.equal(lock.packages[""].devDependencies?.[name], undefined, name);
  }
  for (const [name, entry] of Object.entries(lock.packages)) {
    if (!name) continue;
    assert.match(entry.resolved, /^https:\/\//u, name);
    assert.match(entry.integrity, /^sha512-/u, name);
  }
  for (const command of ["build:platform", "check:types", "dev", "build:server", "build:server:release", "test:rust"]) {
    assert.match(manifest.scripts[command], /web-build\/Cargo\.toml -- /u, command);
    assert.doesNotMatch(manifest.scripts[command], /\b(?:vite|npx|npm)\b/u, command);
  }
  assert.match(read("web/xcss-web/Cargo.toml").toString(), /xcss-web = .*csr/u);
  for (const name of ["index", "login"]) {
    const bootstrap = read(`web/xcss-web-${name}.js`).toString();
    assert.match(bootstrap, /startApplication/u);
    assert.match(bootstrap, /pkg\/xczs_web\.js/u);
    assert.match(bootstrap, /pkg\/xczs_web_bg\.wasm/u);
  }
  const webConfig = read("web/rust-web.toml").toString();
  assert.match(webConfig, /^kind = "embedded"$/mu);
  assert.match(webConfig, /^relative_urls = true$/mu);
  assert.match(webConfig, /"@xcss\/web\/" = "vendor\/xcss\/"/u);
  assert.match(webConfig, /"index\.js" = "xcss-web-index\.js"/u);
  assert.match(webConfig, /"login\.js" = "xcss-web-login\.js"/u);

  const vendor = "web/vendor/xcss";
  const digest = bytes => createHash("sha256").update(bytes).digest("hex");
  const sourceBytes = read(`${vendor}/SOURCE.json`);
  assert.equal(digest(sourceBytes), xcssAssetsReceiptSha256, "reviewed XCSS asset receipt");
  const source = JSON.parse(sourceBytes);
  assert.deepEqual(Object.keys(source).sort(), [
    "base_revision", "files", "format", "origin", "package", "source_directory", "version",
  ]);
  assert.equal(source.format, 1);
  assert.equal(source.package, "@xcss/web");
  assert.equal(source.version, "2.0.0");
  assert.equal(source.origin, "https://github.com/isarmg/xcss");
  assert.equal(source.source_directory, "web");
  assert.match(source.base_revision, /^[0-9a-f]{40}$/u);
  assert.equal(source.base_revision, xcssAssetsRevision);
  for (const [name, hash] of Object.entries(source.files)) {
    assert.match(hash, /^[0-9a-f]{64}$/u, name);
    assert.equal(digest(read(`${vendor}/${name}`)), hash, name);
  }
  const fonts = `${vendor}/web-fonts`;
  const provenance = JSON.parse(read(`${fonts}/provenance.json`));
  for (const [name, hash] of Object.entries(provenance.assets)) {
    assert.equal(digest(read(`${fonts}/${name}`)), hash, name);
  }
  assert.equal(provenance.latin.handwriting, false);
  assert.equal(provenance.assets["NORMAL-LICENSE.txt"], provenance.assets["OFL.txt"]);
  for (const page of ["index.html", "login.html"]) {
    assert.match(read(`web/${page}`).toString(), /data-xcss-appearance="content-blocks"/u);
  }
  assert.match(read("xcss-product.toml").toString(), /web_profile = "web-csr-admin"/u);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  validateXcssWeb();
  console.log("Rust/WASM entries, reviewed Git asset snapshot and font provenance verified without node_modules");
}
