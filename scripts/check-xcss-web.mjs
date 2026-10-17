import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));

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
  const source = JSON.parse(read(`${vendor}/SOURCE.json`));
  assert.equal(source.package, "@xcss/web");
  assert.equal(source.npm.version, "1.0.0");
  assert.equal(source.origin, source.npm.resolved);
  assert.equal(source.npm.resolved, "https://github.com/isarmg/xcss/releases/download/v1.0.0/xcss-web-1.0.0.tgz");
  assert.equal(source.npm.integrity, "sha512-QR/zmdzyQkLnFzTTp+EHYnOL9XEe5N95XVUasnDlKqdNWw/nVoY4evq083QTxMED/ElRRYeegEItnTxCUcb7TQ==");
  assert.deepEqual(Object.keys(source.files).sort(), Object.keys(source.npm.archive_files).sort());
  const digest = bytes => createHash("sha256").update(bytes).digest("hex");
  for (const [name, hash] of Object.entries(source.files)) {
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
  console.log("Rust/WASM entries, immutable vendor resources and font provenance verified without node_modules");
}
