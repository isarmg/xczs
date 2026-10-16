import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";

const hooks = registerHooks({
  resolve(specifier, context, next) {
    if (specifier.endsWith("/platform.ts")) return {
      url: `data:text/javascript,${encodeURIComponent("export const t=(_zh,en)=>en;")}`,
      shortCircuit: true,
    };
    return next(specifier, context);
  },
});
const { createActionDialogs } = await import("../../../web/modules/operations/dialogs.ts");
hooks.deregister();

class Element extends EventTarget {
  constructor() {
    super();
    this.isConnected = true;
    this.hidden = false;
    this.disabled = false;
    this.type = "button";
    this.value = "";
    this.classList = { toggle() {} };
    this.elements = new Map();
  }
  querySelector(selector) { return this.elements.get(selector) ?? null; }
  setAttribute(name, value) { this[name] = String(value); }
  removeAttribute(name) { delete this[name]; }
  focus() { document.activeElement = this; }
  select() {}
  showModal() { this.open = true; }
  close(value) {
    this.returnValue = value;
    this.open = false;
    this.dispatchEvent(new Event("close"));
  }
}

function setup(context) {
  const names = ["document", "HTMLElement", "HTMLDialogElement", "HTMLInputElement", "HTMLButtonElement"];
  const previous = names.map(name => [name, Object.getOwnPropertyDescriptor(globalThis, name)]);
  context.after(() => {
    for (const [name, descriptor] of previous) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else delete globalThis[name];
    }
  });
  for (const name of names.slice(1)) globalThis[name] = Element;
  const dialog = new Element();
  const input = new Element();
  const inputGroup = new Element();
  inputGroup.elements.set("label", new Element());
  inputGroup.elements.set("#action-dialog-input", input);
  const cancel = new Element();
  const alternate = new Element();
  alternate.type = "submit";
  alternate.value = "alternate";
  const confirm = new Element();
  confirm.type = "submit";
  confirm.value = "confirm";
  dialog.elements = new Map([
    ["#action-dialog-title", new Element()],
    ["#action-dialog-message", new Element()],
    [".action-dialog-input-group", inputGroup],
    ["#action-dialog-input", input],
    [".action-dialog-cancel", cancel],
    [".action-dialog-alternate", alternate],
    [".action-dialog-confirm", confirm],
  ]);
  globalThis.document = {
    activeElement: null,
    querySelector: selector => selector === ".action-dialog" ? dialog : null,
  };
  const dialogs = createActionDialogs();
  // Native implicit submission selects the first submit button in tree order,
  // including hidden buttons. The browser test exercises the actual Enter key.
  const defaultSubmitter = () => [cancel, alternate, confirm].find(button => button.type === "submit");
  return { dialogs, dialog, input, alternate, confirm, defaultSubmitter };
}

const shown = () => new Promise(resolve => setImmediate(resolve));

test("move prompts use Confirm as their native default submitter", async context => {
  const { dialogs, dialog, alternate, confirm, defaultSubmitter } = setup(context);
  const result = dialogs.requestText({ title: "Move item", value: "/会议资料 & 计划" });
  await shown();
  assert.equal(alternate.hidden, true);
  assert.equal(alternate.type, "button");
  assert.equal(defaultSubmitter(), confirm);
  dialog.close(defaultSubmitter().value);
  assert.equal(await result, "/会议资料 & 计划");
});

test("choice alternate actions remain available and do not leak into later prompts", async context => {
  const { dialogs, dialog, alternate, confirm, defaultSubmitter } = setup(context);
  const choice = dialogs.chooseAction({ title: "Upload conflict" });
  await shown();
  assert.equal(alternate.hidden, false);
  assert.equal(alternate.type, "submit");
  dialog.close(alternate.value);
  assert.equal(await choice, "alternate");

  const prompt = dialogs.requestText({ title: "Move item", value: "/资料" });
  await shown();
  assert.equal(alternate.hidden, true);
  assert.equal(alternate.type, "button");
  assert.equal(defaultSubmitter(), confirm);
  dialog.close(confirm.value);
  assert.equal(await prompt, "/资料");

  const cancelled = dialogs.requestText({ title: "Move item", value: "/资料" });
  await shown();
  dialog.close("cancel");
  assert.equal(await cancelled, null);
});
