import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const fixtureScript = readFileSync(new URL("../language.mjs", import.meta.url), "utf8");
for (const [name, declaration, pageUrl] of [
  ["index", "html", "http://xczs-language.test/?lang=zh-CN#files"],
  ["login", "loginHtml", "http://xczs-language.test/__xczs__/login?lang=zh-CN"],
]) {
  test(`${name} actual HTML and fixture prefix resolve every resource to the same origin`, () => {
    // Read the actual replacement used by the browser script, not a second
    // hand-authored renderer. The two production templates have distinct prefixes.
    const expression = fixtureScript.split(`const ${declaration} = `)[1]?.split(";\n")[0];
    const prefix = expression?.match(/\.replaceAll\('__ASSETS_PREFIX__', '([^']+)'\)/u)?.[1];
    assert.equal(typeof prefix, "string");
    const source = readFileSync(new URL(`../../../web/${name}.html`, import.meta.url), "utf8");
    const html = source.replaceAll("__ASSETS_PREFIX__", prefix);
    const resources = [...html.matchAll(/(?:src|href)="([^"]+)"/gu)].map(match => match[1]);
    assert(resources.length >= 6);
    assert(resources.includes(`/__xczs__/${name}.js`));
    for (const resource of resources) {
      assert(!resource.startsWith("//"), resource);
      const url = new URL(resource, pageUrl);
      assert.equal(url.origin, new URL(pageUrl).origin, resource);
      assert(url.pathname.startsWith("/__xczs__/"), resource);
      assert(!url.pathname.slice("/__xczs__/".length).includes("__xczs__"), resource);
    }
  });
}
