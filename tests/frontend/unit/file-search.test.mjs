import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { transformSync } from "rolldown/utils";

// Execute the actual component, with only React's hook scheduling, JSX output,
// and the network boundary replaced. Browser coverage checks the mounted pane.
const modules = {
  react: "export const useState=(...args)=>globalThis.__SEARCH_HOOKS.useState(...args);export const useEffect=(...args)=>globalThis.__SEARCH_HOOKS.useEffect(...args);export const useRef=()=>{};export const useSyncExternalStore=()=>{};",
  "react/jsx-runtime": 'export const jsx=(type,props)=>({type,props});export const jsxs=jsx;export const Fragment="Fragment";',
  "@xcss/web/admin-ui": 'export const Button="Button",ErrorState="ErrorState",FormField="FormField",LoadingState="LoadingState",Select="Select";',
  "@xcss/web/admin-ui/i18n": "export const t=(_zh,en)=>en;",
  "./tag-api.ts": "export const tagRequest=(...args)=>globalThis.__SEARCH_REQUEST(...args);export const tagFailure=String;",
  "../modules/listing/action-mode.ts": "export const fileActionState=()=>({});export const subscribeFileAction=()=>()=>{};",
};
const loader = registerHooks({
  resolve(specifier, context, next) {
    if (Object.hasOwn(modules, specifier)) return {
      url: `data:text/javascript,${encodeURIComponent(modules[specifier])}`,
      shortCircuit: true,
    };
    return next(specifier, context);
  },
  load(url, context, next) {
    if (url.endsWith("/directory-tags.tsx")) return {
      format: "module",
      source: transformSync("directory-tags.tsx", readFileSync(new URL(url), "utf8"), {
        jsx: { runtime: "automatic" },
      }).code,
      shortCircuit: true,
    };
    return next(url, context);
  },
});
const { FileSearch } = await import("../../../web/react/directory-tags.tsx");
loader.deregister();

function setup(context, search = "") {
  const names = ["window", "__SEARCH_HOOKS", "__SEARCH_REQUEST"];
  const previous = Object.fromEntries(names.map(name => [name, Object.getOwnPropertyDescriptor(globalThis, name)]));
  const states = [], effects = [], pending = [], requests = [];
  let index = 0, dirty = false, tree, deferred;
  const client = {};
  let response = { tags: [], previous_cursor: null, next_cursor: null };
  globalThis.window = new EventTarget();
  window.location = { search };
  globalThis.__SEARCH_HOOKS = {
    useState(initial) {
      const slot = index++;
      if (!(slot in states)) states[slot] = typeof initial === "function" ? initial() : initial;
      return [states[slot], value => {
        const next = typeof value === "function" ? value(states[slot]) : value;
        if (!Object.is(next, states[slot])) { states[slot] = next; dirty = true; }
      }];
    },
    useEffect(callback, dependencies) {
      const slot = index++;
      const previous = effects[slot];
      if (!previous || dependencies.some((value, at) => !Object.is(value, previous.dependencies[at]))) {
        const effect = { dependencies };
        effects[slot] = effect;
        pending.push(() => { previous?.cleanup?.(); effect.cleanup = callback(); });
      }
    },
  };
  globalThis.__SEARCH_REQUEST = async (_client, path, _guard, _method, _body, signal) => {
    requests.push(path);
    if (deferred) {
      const request = deferred;
      deferred = undefined;
      request.signal = signal;
      return await request.promise;
    }
    return response;
  };
  const render = () => {
    index = 0;
    dirty = false;
    tree = FileSearch({ client });
    for (const effect of pending.splice(0)) effect();
  };
  const flush = async () => {
    for (let count = 0; count < 10; count++) {
      await new Promise(resolve => setImmediate(resolve));
      if (!dirty) return;
      render();
    }
    assert.fail("FileSearch did not settle");
  };
  const unmount = () => effects.forEach(effect => effect?.cleanup?.());
  context.after(() => {
    unmount();
    for (const name of names) {
      if (previous[name]) Object.defineProperty(globalThis, name, previous[name]);
      else delete globalThis[name];
    }
  });
  function nodes(type) {
    const result = [];
    const visit = node => {
      if (!node || typeof node !== "object") return;
      if (node.type === type) result.push(node);
      for (const child of [node.props?.children].flat(Infinity)) visit(child);
    };
    visit(tree);
    return result;
  }
  render();
  return {
    requests, flush, unmount, nodes,
    isDirty: () => dirty,
    deferNext() {
      const request = {};
      request.promise = new Promise(resolve => { request.resolve = resolve; });
      deferred = request;
      return request;
    },
    respond(tags, next_cursor = null) { response = { tags, previous_cursor: null, next_cursor }; },
    async open(open = true) {
      nodes("details")[0].props.onToggle({ currentTarget: { open } });
      await flush();
    },
    async changed() { window.dispatchEvent(new Event("xczs:tags-changed")); await flush(); },
    select: name => nodes("Select").find(node => node.props.name === name),
  };
}
const tag = (id, name) => ({ id, name, color: null, file_count: 0 });
const labels = select => [select.props.children].flat(Infinity).map(option => [option.props.children].flat(Infinity).join(""));

test("an open filter picker refreshes created and renamed tags without changing chosen conditions", async context => {
  const view = setup(context, "?q=report&all=1&any=2&exclude=7");
  view.respond([tag(1, "Before")]);
  await view.open();
  view.respond([tag(1, "Renamed"), tag(2, "Created")]);
  await view.changed();
  assert.deepEqual(view.requests, ["/tags", "/tags"]);
  assert.deepEqual(labels(view.select("all")), ["Renamed", "Created"]);
  assert.deepEqual(view.select("all").props.value, ["1"]);
  assert.deepEqual(view.select("any").props.value, ["2"]);
  assert.deepEqual(view.select("exclude").props.value, ["7"]);
  assert.equal(window.location.search, "?q=report&all=1&any=2&exclude=7");
});

test("tag changes restart candidate pagination and leave deleted selected IDs visible and removable", async context => {
  const view = setup(context, "?all=1");
  view.respond([tag(1, "Deleted later")], "page-two");
  await view.open();
  view.respond([tag(2, "Second page")]);
  view.nodes("Button").find(node => node.props.children === "Next page").props.onClick();
  await view.flush();
  assert.equal(view.requests[1], "/tags?cursor=page-two");
  view.respond([tag(2, "Remaining")]);
  await view.changed();
  assert.equal(view.requests[2], "/tags");
  assert.deepEqual(view.select("all").props.value, ["1"]);
  assert.deepEqual(labels(view.select("all")), ["#1", "Remaining"]);
  view.select("all").props.onChange({ currentTarget: { selectedOptions: [] } });
  await view.flush();
  assert.deepEqual(view.select("all").props.value, []);
});

test("closed filter pickers defer fetching and remove mutation listeners on unmount", async context => {
  const view = setup(context);
  view.respond([tag(1, "Current")]);
  await view.changed();
  assert.deepEqual(view.requests, []);
  await view.open();
  assert.deepEqual(view.requests, ["/tags"]);
  await view.open(false);
  await view.changed();
  assert.deepEqual(view.requests, ["/tags"]);
  await view.open();
  assert.deepEqual(view.requests, ["/tags", "/tags"]);
  view.unmount();
  window.dispatchEvent(new Event("xczs:tags-changed"));
  window.dispatchEvent(new Event("popstate"));
  assert.equal(view.isDirty(), false);
});


test("superseded tag requests cannot restore old candidates after a mutation", async context => {
  const view = setup(context, "?all=1");
  const old = view.deferNext();
  await view.open();
  view.respond([tag(2, "Current")]);
  await view.changed();
  assert.equal(old.signal.aborted, true);
  assert.deepEqual(labels(view.select("all")), ["#1", "Current"]);
  old.resolve({ tags: [tag(1, "Deleted")], previous_cursor: null, next_cursor: "obsolete" });
  await view.flush();
  assert.deepEqual(labels(view.select("all")), ["#1", "Current"]);
  assert.equal(view.nodes("Button").find(node => node.props.children === "Next page").props.disabled, true);
});

for (const action of ["close", "unmount"]) {
  test(`tag requests cannot write state after ${action}`, async context => {
    const view = setup(context);
    const request = view.deferNext();
    await view.open();
    if (action === "close") await view.open(false);
    else view.unmount();
    assert.equal(request.signal.aborted, true);
    request.resolve({ tags: [tag(1, "Late")], previous_cursor: null, next_cursor: null });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(view.isDirty(), false);
    assert.deepEqual(labels(view.select("all")), []);
  });
}
