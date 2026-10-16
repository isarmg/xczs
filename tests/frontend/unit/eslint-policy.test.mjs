import assert from "node:assert/strict";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { ESLint } from "eslint";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const eslint = new ESLint({ cwd: root });
const bootstraps = ["web/leptos-index.js", "web/leptos-login.js", "web/leptos-platform.js"];

async function ruleIds(source, filePath) {
  const [result] = await eslint.lintText(source, { filePath: resolve(root, filePath) });
  return result.messages.map(message => message.ruleId);
}

test("all Leptos bootstrap entries reject browser injection and dynamic code", async () => {
  const probes = [
    ["element.innerHTML = userInput;\n", "nounsanitized/property"],
    ["document.write(userInput);\n", "nounsanitized/method"],
    ["eval(userInput);\n", "no-eval"],
    ["alert(userInput);\n", "no-alert"],
    ["new Function(userInput);\n", "no-new-func"],
    ["setTimeout('alert(1)', 1);\n", "no-implied-eval"],
    ["h('div', { dangerouslySetInnerHTML: value });\n", "no-restricted-syntax"],
    ["new DOMParser();\n", "no-restricted-syntax"],
    ["range.createContextualFragment(userInput);\n", "no-restricted-syntax"],
    ["element.setHTMLUnsafe(userInput);\n", "no-restricted-syntax"],
  ];
  for (const filePath of bootstraps) {
    for (const [source, rule] of probes) {
      assert((await ruleIds(source, filePath)).includes(rule), `${filePath}: ${rule}`);
    }
  }
});

test("Leptos bootstrap network requests remain owned by Rust", async () => {
  const probes = [
    ["fetch('/');\n", "no-restricted-globals"],
    ["new XMLHttpRequest();\n", "no-restricted-globals"],
    ["window.fetch('/');\n", "no-restricted-syntax"],
    ["globalThis.fetch('/');\n", "no-restricted-syntax"],
    ["new window.XMLHttpRequest();\n", "no-restricted-syntax"],
    ["new globalThis.XMLHttpRequest();\n", "no-restricted-syntax"],
  ];
  for (const filePath of bootstraps) {
    for (const [source, rule] of probes) {
      assert((await ruleIds(source, filePath)).includes(rule), `${filePath}: ${rule}`);
    }
  }
});

test("the three actual production bootstrap files pass the same active rules", async () => {
  const results = await eslint.lintFiles(bootstraps);
  assert.equal(results.length, bootstraps.length);
  for (const result of results) {
    assert.equal(result.errorCount, 0, JSON.stringify(result.messages));
    assert.equal(result.warningCount, 0, JSON.stringify(result.messages));
  }
});
