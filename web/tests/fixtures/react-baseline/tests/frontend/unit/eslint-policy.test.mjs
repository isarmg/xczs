import assert from "node:assert/strict";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { ESLint } from "eslint";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const eslint = new ESLint({ cwd: root });

async function ruleIds(source, filePath = "web/modules/listing/controller.ts") {
  const [result] = await eslint.lintText(source, { filePath: resolve(root, filePath) });
  return result.messages.map(message => message.ruleId);
}

test("ESLint rejects common browser injection and dynamic-code APIs", async () => {
  assert((await ruleIds("element.innerHTML = userInput;\n")).includes("nounsanitized/property"));
  assert((await ruleIds("document.write(userInput);\n")).includes("nounsanitized/method"));
  assert((await ruleIds("eval(userInput);\n")).includes("no-eval"));
  assert((await ruleIds("alert(userInput);\n")).includes("no-alert"));
  assert((await ruleIds("h('div', { dangerouslySetInnerHTML: value });\n"))
    .includes("no-restricted-syntax"));
});

test("ESLint keeps network primitives in their owning modules", async () => {
  assert((await ruleIds("fetch('/');\n")).includes("no-restricted-globals"));
  assert((await ruleIds("new XMLHttpRequest();\n")).includes("no-restricted-globals"));
  assert.equal(
    (await ruleIds("fetch('/');\n", "web/modules/http/client.ts")).length,
    0,
  );
  assert.equal(
    (await ruleIds("new XMLHttpRequest();\n", "web/modules/upload/transport.ts")).length,
    0,
  );
});

test("browser safety rules also inspect TypeScript assertions and TSX props", async () => {
  assert((await ruleIds("(element as HTMLElement).innerHTML = userInput;\n"))
    .includes("nounsanitized/property"));
  assert((await ruleIds("const page = <div dangerouslySetInnerHTML={{ __html: userInput }} />;\n", "web/react/tags.tsx"))
    .includes("no-restricted-syntax"));
});
