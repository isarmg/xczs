import assert from "node:assert/strict";
import test from "node:test";
import { registerHooks } from "node:module";

// Run the real upload manager, queue, protocol parser and transport against
// deterministic DOM/XHR boundaries. No browser or external server is needed.
const hooks = registerHooks({
  resolve(specifier, context, next) {
    if (specifier.endsWith("/platform.ts")) return {
      url: `data:text/javascript,${encodeURIComponent('export const t=(_zh,en,values=[])=>en.replace(/\\{(\\d+)\\}/g,(_,i)=>values[+i]); export const getLocale=()=>"en"; export const isErrorEnvelope=()=>false;')}`,
      shortCircuit: true,
    };
    return next(specifier, context);
  },
});
const { createUploadManager } = await import("../../../web/modules/upload/manager.ts");
hooks.deregister();

class Element extends EventTarget {
  constructor(tag = "div") {
    super(); this.tag = tag; this.children = []; this.attributes = new Map();
    this.classList = { add() {}, remove() {}, toggle() {} };
  }
  setAttribute(name, value) { this.attributes.set(name, String(value)); }
  removeAttribute(name) { this.attributes.delete(name); }
  hasAttribute(name) { return this.attributes.has(name); }
  append(...children) { this.children.push(...children); for (const child of children) child.parent = this; }
  replaceChildren(...children) { this.children = []; this.append(...children); }
  contains(node) { return node === this || this.children.some(child => child.contains(node)); }
  get isConnected() { return Boolean(this.parent); }
  remove() { if (this.parent) this.parent.children = this.parent.children.filter(child => child !== this); this.parent = null; }
  querySelector(selector) {
    const descendants = this.children.flatMap(child => [child, ...child.descendants()]);
    return descendants.find(child => selector === ".retry-btn" ? child.className === "retry-btn" : child.tag === "a") ?? null;
  }
  descendants() { return this.children.flatMap(child => [child, ...child.descendants()]); }
}

function setup(context, concurrency = 2, chooseAction = async () => "cancel") {
  const names = ["window", "document", "location", "HTMLElement", "HTMLTableRowElement", "HTMLAnchorElement", "XMLHttpRequest", "fetch"];
  const previous = Object.fromEntries(names.map(name => [name, Object.getOwnPropertyDescriptor(globalThis, name)]));
  const requests = [];
  class Xhr extends EventTarget {
    static HEADERS_RECEIVED = 2;
    constructor() { super(); this.upload = new EventTarget(); this.headers = new Headers(); this.responseHeaders = new Headers(); this.status = 0; this.readyState = 0; this.responseText = ""; }
    open(method, url) { this.method = method; this.url = url; }
    setRequestHeader(name, value) { this.headers.set(name, value); }
    getResponseHeader(name) { return this.responseHeaders.get(name); }
    send(body) { this.body = body; requests.push(this); }
    abort() { this.dispatchEvent(new Event("abort")); }
    failNetwork() { this.dispatchEvent(new Event("error")); }
    expireSession() {
      this.status = 401;
      this.readyState = Xhr.HEADERS_RECEIVED;
      this.dispatchEvent(new Event("readystatechange"));
    }
    complete() {
      this.status = this.method === "PATCH" ? 204 : 201;
      this.responseHeaders = new Headers({
        "X-Xczs-Upload-Id": this.headers.get("X-Xczs-Upload-Id"),
        "X-Xczs-Upload-Length": this.headers.get("X-Xczs-Upload-Length"),
        "X-Xczs-Upload-Offset": this.headers.get("X-Xczs-Upload-Length"),
        "X-Xczs-Operation-State": "committed",
      });
      this.dispatchEvent(new Event("load"));
    }
  }
  const timers = new Map(); let timerId = 0;
  globalThis.window = Object.assign(new EventTarget(), {
    setTimeout(callback, delay) {
      const id = ++timerId;
      timers.set(id, callback);
      if (delay === 0) queueMicrotask(() => { if (timers.delete(id)) callback(); });
      return id;
    },
    clearTimeout(id) { timers.delete(id); },
    // A hidden document does not deliver animation frames.
    requestAnimationFrame() { return 1; },
  });
  globalThis.document = { activeElement: null, createElement: tag => new Element(tag), createElementNS: (_ns, tag) => new Element(tag) };
  globalThis.location = { href: "https://example.test/docs/", origin: "https://example.test" };
  for (const name of ["HTMLElement", "HTMLTableRowElement", "HTMLAnchorElement"]) globalThis[name] = Element;
  globalThis.XMLHttpRequest = Xhr;
  globalThis.fetch = async (_url, options) => new Response(JSON.stringify({
    targets: JSON.parse(options.body).paths.map(path => ({ path, exists: false, revision: null, replaceable: true })),
  }), { headers: { "Content-Type": "application/json" } });
  const table = new Element("table"); table.tBodies = [new Element("tbody")]; table.append(table.tBodies[0]);
  const mutations = [], unauthorizedWarnings = [];
  const manager = createUploadManager({
    data: { href: "/docs", dir_exists: true, session: { csrf_token: "test-token" } },
    dialogs: { showMessage: async () => {}, chooseAction },
    uploadersTable: table, queueMessage: new Element(), historyStatus: new Element(), emptyFolder: new Element(),
    onMutation: effect => mutations.push(effect), onUnauthorized() { unauthorizedWarnings.push(shouldWarn()); }, maxConcurrentUploads: concurrency,
  });
  const shouldWarn = () => {
    const event = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(event);
    return event.defaultPrevented;
  };
  context.after(() => {
    for (const name of names) { if (previous[name]) Object.defineProperty(globalThis, name, previous[name]); else delete globalThis[name]; }
  });
  return { manager, requests, shouldWarn, mutations, table, timers, unauthorizedWarnings };
}

test("an uncertain upload does not remove protection for another running transfer", async context => {
  const {manager, requests, shouldWarn, mutations} = setup(context);
  assert.equal(shouldWarn(), false);
  await manager.addFiles([new File(["one"], "one.txt"), new File(["two"], "two.txt"), new File(["three"], "three.txt")]);
  assert.equal(requests.length, 2);
  assert.equal(shouldWarn(), true);
  requests[0].failNetwork();
  assert.equal(requests.length, 2, "a paused queue must not dispatch its third upload");
  assert.equal(shouldWarn(), true, "the second upload is still transferring");
  requests[1].complete();
  assert.deepEqual(mutations, ["outcome-unknown", "committed"]);
  assert.equal(requests.length, 2);
  assert.equal(shouldWarn(), false, "the user may refresh when only a paused queue remains");
});

test("sending the complete body keeps the unload warning until server confirmation", async context => {
  const {manager, requests, shouldWarn, mutations, table, timers} = setup(context, 1);
  await manager.addFiles([new File(["one"], "one.txt"), new File([], "empty.txt")]);
  assert.equal(requests.length, 1);
  requests[0].upload.dispatchEvent(new Event("load"));
  assert.equal(shouldWarn(), true);
  assert.deepEqual(mutations, []);
  assert.match(table.tBodies[0].children[0].children[2].attributes.get("aria-label"), /waiting for server confirmation/);
  requests[0].complete();
  assert.equal(requests.length, 2, "confirmed completion frees the next queue slot");
  assert.equal(requests[1].body.size, 0);
  assert.equal(requests[1].headers.get("X-Xczs-Upload-Length"), "0");
  assert.equal(shouldWarn(), true);
  requests[1].complete();
  assert.deepEqual(mutations, ["committed", "committed"]);
  assert.equal(shouldWarn(), false);
  assert.equal(manager.isBusy(), false);
  assert.equal(timers.size, 0, "completion clears transfer and commit timers");
});

test("an uncertain single upload allows refresh without dispatching its paused queue", async context => {
  const {manager, requests, shouldWarn} = setup(context, 1);
  await manager.addFiles([new File(["one"], "one.txt"), new File(["two"], "two.txt")]);
  requests[0].failNetwork();
  assert.equal(requests.length, 1);
  assert.equal(shouldWarn(), false);
});


test("folder batches continue admission when a hidden document has no animation frames", async context => {
  const {manager, requests, table} = setup(context, 1);
  const files = Array.from({length: 51}, (_, index) => new File(["content"], `${index}.txt`));
  let admitted = false;
  const addition = manager.addFiles(files).then(() => { admitted = true; });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(table.tBodies[0].children.length, 51, "all files must be admitted without a visibility change");
  assert.equal(admitted, true);
  await addition;
  for (let index = 0; index < files.length; index++) {
    assert.equal(requests.length, index + 1);
    requests[index].complete();
  }
  assert.equal(manager.isBusy(), false);
});


test("an expired-session response protects other active transfers during navigation", async context => {
  const {manager, requests, shouldWarn, unauthorizedWarnings} = setup(context, 2);
  await manager.addFiles([new File(["one"], "one.txt"), new File(["two"], "two.txt"), new File(["three"], "three.txt")]);
  requests[0].expireSession();
  assert.deepEqual(unauthorizedWarnings, [true], "navigation must account for the unrelated active upload");
  assert.equal(shouldWarn(), true);
  assert.equal(requests.length, 2);
  requests[1].complete();
  assert.equal(shouldWarn(), false);
  assert.equal(requests.length, 2, "remaining uploads stay paused");
});

test("an expired-session response with no other active transfer navigates without warning", async context => {
  const {manager, requests, shouldWarn, unauthorizedWarnings} = setup(context, 1);
  await manager.addFiles([new File(["one"], "one.txt"), new File(["two"], "two.txt")]);
  requests[0].expireSession();
  assert.deepEqual(unauthorizedWarnings, [false], "a finished rejected request must not count as an active transfer");
  assert.equal(shouldWarn(), false);
  assert.equal(requests.length, 1);
});


function uploadError(request, status, code, state, offset, recovery, targetHeaders = {}) {
  request.status = status;
  request.responseHeaders = new Headers({
    "Content-Type": "application/problem+json",
    "X-Xczs-Upload-Id": request.headers.get("X-Xczs-Upload-Id"),
    "X-Xczs-Upload-Length": request.headers.get("X-Xczs-Upload-Length"),
    "X-Xczs-Upload-Offset": String(offset),
    "X-Xczs-Operation-State": state,
    ...targetHeaders,
  });
  request.responseText = JSON.stringify({ status, code, detail: code, recovery });
  request.dispatchEvent(new Event("load"));
}

for (const offset of [0, 2, 4]) {
  for (const destination of ["changed", "removed"]) {
    test(`a resumed overwrite recovers when its destination is ${destination} at offset ${offset}`, async context => {
      const dialogs = [];
      const {manager, requests, table, mutations} = setup(context, 1, async options => {
        dialogs.push(options);
        return "confirm";
      });
      const originalRevision = "a".repeat(64);
      const currentRevision = "b".repeat(64);
      globalThis.fetch = async (_url, options) => {
        if (options.method === "HEAD") return new Response(null, { headers: {
          "X-Xczs-Upload-Id": new Headers(options.headers).get("X-Xczs-Upload-Id"),
          "X-Xczs-Upload-Length": "4",
          "X-Xczs-Upload-Offset": String(offset),
          "X-Xczs-Operation-State": "running",
        } });
        return new Response(JSON.stringify({ targets: JSON.parse(options.body).paths.map(path => ({
          path, exists: true, revision: originalRevision, replaceable: true,
        })) }), { headers: { "Content-Type": "application/json" } });
      };
      await manager.addFiles([new File(["data"], "one.txt")]);
      uploadError(requests[0], 408, "upload_idle_timeout", "running", offset, "resume_upload");
      table.querySelector(".retry-btn").dispatchEvent(new Event("click"));
      await new Promise(resolve => setImmediate(resolve));
      assert.equal(requests[1].method, "PATCH");
      uploadError(requests[1], 409, destination === "changed" ? "destination_exists" : "upload_target_changed",
        "running", offset, "refresh_target", {
          "X-Xczs-Target-Replaceable": "true",
          ...(destination === "changed" ? { "X-Xczs-Target-Revision": currentRevision } : {}),
        });
      await new Promise(resolve => setImmediate(resolve));
      assert.equal(dialogs.length, destination === "changed" ? 2 : 1);
      assert.equal(requests.length, 3, "the retained checkpoint must remain resumable after the target conflict");
      const resumed = requests[2];
      assert.equal(resumed.method, "PATCH", "even a zero-offset checkpoint is an existing session");
      assert.equal(resumed.headers.get("X-Xczs-Upload-Id"), requests[0].headers.get("X-Xczs-Upload-Id"));
      assert.equal(resumed.headers.get("X-Xczs-Upload-Offset"), String(offset));
      assert.equal(resumed.headers.get("X-Xczs-Target-Revision"), destination === "changed" ? currentRevision : null);
      assert.equal(resumed.headers.get("X-Xczs-Upload-Overwrite"), String(destination === "changed"));
      assert.equal(await resumed.body.text(), "data".slice(offset));
      assert.ok(mutations.includes("refresh-required"));
      resumed.complete();
      assert.equal(manager.isBusy(), false);
    });
  }
}
