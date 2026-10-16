import assert from "node:assert/strict";
import test from "node:test";
import { registerHooks } from "node:module";

// Minimal DOM boundary for the actual directory controller. No implementation is
// copied: refresh, request generation and table-header rendering run unchanged.
class Element extends EventTarget {
  constructor(tag = "div") {
    super();
    this.tag = tag;
    this.children = [];
    this.attributes = {};
    this.dataset = {};
    this.classes = new Set();
    this.classList = {
      add: (...names) => names.forEach(name => this.classes.add(name)),
      remove: (...names) => names.forEach(name => this.classes.delete(name)),
      toggle: (name, present) => {
        const value = present ?? !this.classes.has(name);
        if (value) this.classes.add(name); else this.classes.delete(name);
        return value;
      },
    };
  }
  setAttribute(name, value) { this.attributes[name] = String(value); }
  getAttribute(name) { return this.attributes[name] ?? null; }
  append(...elements) { this.children.push(...elements); }
  before() {}
  replaceChildren(...elements) { this.children = elements; }
  get rows() { return this.children; }
  contains(element) { return this.children.includes(element); }
}
const hooks = registerHooks({
  resolve(specifier, context, next) {
    if (specifier.endsWith("/platform.ts")) return {
      url: `data:text/javascript,${encodeURIComponent('export const t=(_zh,en,values=[])=>en.replace(/\\{(\\d+)\\}/g,(_,i)=>values[+i]);')}`,
      shortCircuit: true,
    };
    if (specifier === "../http/client.ts") return {
      url: `data:text/javascript,${encodeURIComponent('export class RequestError extends Error{};export const assertResponse=async()=>{};export const isAuthenticationError=()=>false;export const requestJson=(...args)=>globalThis.__LISTING_REQUEST(...args);')}`,
      shortCircuit: true,
    };
    return next(specifier, context);
  },
});
const { createDirectoryListing } = await import("../../../web/modules/listing/controller.ts");
hooks.deregister();
function setup(context) {
  const globals = ["window", "document", "location", "Element", "HTMLElement", "HTMLTableRowElement", "Node", "__LISTING_REQUEST"];
  const previous = Object.fromEntries(globals.map(name => [name, Object.getOwnPropertyDescriptor(globalThis, name)]));
  const table = new Element("table"), tableHead = new Element("thead"), tableBody = new Element("tbody");
  globalThis.window = new EventTarget();
  globalThis.document = { activeElement: null, createElement: tag => new Element(tag), createDocumentFragment: () => new Element("fragment") };
  globalThis.location = { origin: "https://example.test", href: "https://example.test/docs/" };
  for (const name of ["Element", "HTMLElement", "HTMLTableRowElement", "Node"]) globalThis[name] = Element;
  const requests = [];
  globalThis.__LISTING_REQUEST = url => new Promise((resolve, reject) => requests.push({ url, reject, resolve() { resolve({ response: {status:200}, payload: {paths:[], next_cursor:null, file_tags:[]} }); } }));
  const params = { q:"", all:"", any:"", exclude:"", sort:"name", order:"asc" };
  const listing = createDirectoryListing({ data:{href:"/docs", dir_exists:true, session:{}}, params, table, tableHead, tableBody, emptyFolder:new Element(), emptyNote:"Empty", loadMore:new Element("button"), listStatus:new Element(), onAction(){}, onRename(){}, onUnauthorized(){}, onTags(){} });
  context.after(() => {
    for (const name of globals) { if (previous[name]) Object.defineProperty(globalThis, name, previous[name]); else delete globalThis[name]; }
  });
  const header = name => tableHead.children[0].children.find(cell => cell.className === `cell-${name}`);
  const sortUrl = name => new URL(header(name).children[0].getAttribute("href"), location.href);
  return { listing, params, requests, header, sortUrl };
}

test("in-place filters and browser-history changes are preserved by rebuilt sort links", async context => {
  const {listing, params, requests, sortUrl, header} = setup(context);
  let loading = listing.loadNextPage(); requests.at(-1).resolve(); await loading;
  Object.assign(params, {q:"budget & 中文", all:"7", any:"8", exclude:"9"});
  loading = listing.refreshFromFirstPage();
  for (const key of ["q","all","any","exclude"]) assert.equal(sortUrl("size").searchParams.get(key), params[key]);
  assert.equal(sortUrl("size").searchParams.get("sort"), "size");
  assert.equal(requests.at(-1).url.searchParams.get("all"), "7");
  requests.at(-1).resolve(); await loading;
  Object.assign(params, {q:"", all:"", any:"", exclude:"", sort:"size", order:"desc"});
  loading = listing.refreshFromFirstPage();
  for (const key of ["q","all","any","exclude"]) assert.equal(sortUrl("mtime").searchParams.has(key), false);
  assert.equal(header("size").getAttribute("aria-sort"), "descending");
  assert.equal(sortUrl("size").searchParams.get("order"), "asc");
  requests.at(-1).reject(new Error("Temporary list failure")); await loading;
  assert.equal(sortUrl("size").searchParams.get("order"), "asc");
});

test("filter changes rebuild sorting even while an obsolete list request is still pending", async context => {
  const {listing, params, requests, sortUrl} = setup(context);
  const first = listing.loadNextPage();
  Object.assign(params, {q:"latest", all:"7"});
  await listing.refreshFromFirstPage();
  assert.equal(sortUrl("size").searchParams.get("q"), "latest");
  requests[0].resolve(); await first;
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(requests.length, 2);
  assert.equal(requests[1].url.searchParams.get("q"), "latest");
  requests[1].resolve(); await new Promise(resolve => setImmediate(resolve));
  assert.equal(sortUrl("size").searchParams.get("all"), "7");
});
