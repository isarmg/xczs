import assert from "node:assert/strict";
import test from "node:test";
import { ApiClientError } from "@xcss/web/http-client";
import { tagRequest, tagFailure, TAG_SCAN_TIMEOUT_MS } from "../../../web/react/tag-api.ts";

const busy = () => new ApiClientError({
  message: "Busy",
  status: 429,
  code: "too_many_requests",
  retryable: true,
  retryAfterSeconds: 0,
});
const accepted = value => value === "accepted";

test("temporary tag reads retry before admitting the next request", async () => {
  const requests = [];
  let reads = 0;
  const client = {
    async request(path) {
      requests.push(path);
      if (path.endsWith("/tags") && reads++ === 0) throw busy();
      return "accepted";
    },
  };
  await Promise.all([
    tagRequest(client, "/tags", accepted),
    tagRequest(client, "/status", accepted),
  ]);
  assert.deepEqual(requests, [
    "/api/v1/file-tags/tags",
    "/api/v1/file-tags/tags",
    "/api/v1/file-tags/status",
  ]);
});

test("tag retries are bounded and never repeat writes or permanent failures", async () => {
  let requests = 0;
  const client = {
    async request() {
      requests++;
      throw busy();
    },
  };
  await assert.rejects(tagRequest(client, "/tags", accepted));
  assert.equal(requests, 3);
  requests = 0;
  await assert.rejects(tagRequest(client, "/tags", accepted, "POST", {}));
  assert.equal(requests, 1);
  requests = 0;
  client.request = async () => {
    requests++;
    throw new ApiClientError({
      message: "Capacity reached",
      status: 503,
      code: "capacity_exhausted",
    });
  };
  await assert.rejects(tagRequest(client, "/tags", accepted));
  assert.equal(requests, 1);
});

test("obsolete queued tag reads do not issue requests", async () => {
  let release;
  const gate = new Promise(resolve => { release = resolve; });
  const requests = [];
  const client = {
    async request(path) {
      requests.push(path);
      await gate;
      return "accepted";
    },
  };
  const first = tagRequest(client, "/tags", accepted);
  const controller = new AbortController();
  const next = tagRequest(client, "/status", accepted, undefined, undefined, controller.signal);
  const cancelled = assert.rejects(next, { name: "AbortError" });
  controller.abort();
  release();
  await Promise.all([first, cancelled]);
  assert.deepEqual(requests, ["/api/v1/file-tags/tags"]);
});

test("view changes cancel dispatched reads and release the serial queue", async () => {
  const controller = new AbortController();
  let dispatched;
  const started = new Promise(resolve => { dispatched = resolve; });
  const requests = [];
  const client = { request(path, guard, options) {
    requests.push(path);
    if (path.endsWith("/status")) return Promise.resolve("accepted");
    assert.equal(options.signal, controller.signal);
    dispatched();
    return new Promise((resolve, reject) => options.signal.addEventListener("abort", () =>
      reject(new DOMException("View changed", "AbortError")), { once: true }));
  } };
  const first = tagRequest(client, "/tags", accepted, undefined, undefined, controller.signal);
  const rejected = assert.rejects(first, { name: "AbortError" });
  const next = tagRequest(client, "/status", accepted);
  await started;
  controller.abort();
  await rejected;
  assert.equal(await next, "accepted");
  assert.deepEqual(requests, ["/api/v1/file-tags/tags", "/api/v1/file-tags/status"]);
});

test("full scans have a dedicated budget beyond collection and never retry on timeout", async () => {
  let requests = 0;
  const client = { async request(path, guard, options) {
    requests++;
    assert.equal(path, "/api/v1/file-tags/scan");
    assert.equal(options.timeoutMs, TAG_SCAN_TIMEOUT_MS);
    assert.ok(options.timeoutMs > 30_000);
    throw new ApiClientError({ message: "Unconfirmed scan", code: "request_timeout", retryable: true });
  } };
  await assert.rejects(tagRequest(client, "/scan", accepted, "POST"));
  assert.equal(requests, 1);
});

test("aborting a read during Retry-After releases the queue without waiting or retrying", async () => {
  const controller = new AbortController();
  let reading;
  const started = new Promise(resolve => { reading = resolve; });
  const calls = [];
  const client = { async request(path) {
    calls.push(path);
    if (path.endsWith("/status")) return "accepted";
    reading();
    throw new ApiClientError({ message: "Busy", status: 429, code: "too_many_requests", retryable: true, retryAfterSeconds: 2 });
  } };
  const first = tagRequest(client, "/tags", accepted, undefined, undefined, controller.signal);
  const cancelled = assert.rejects(first, { name: "AbortError" });
  const next = tagRequest(client, "/status", accepted);
  await started;
  await new Promise(resolve => setImmediate(resolve));
  controller.abort();
  const timeout = setTimeout(() => assert.fail("Cancelled Retry-After still blocks new views"), 500);
  try { await cancelled; assert.equal(await next, "accepted"); }
  finally { clearTimeout(timeout); }
  assert.deepEqual(calls, ["/api/v1/file-tags/tags", "/api/v1/file-tags/status"]);
});


test("file tag limit has an actionable error message", () => {
  const message = tagFailure(new ApiClientError({message: "Limit", status: 409, code: "tag_limit_exceeded"}));
  assert.match(message, /16/);
  assert.doesNotMatch(message, /Could not read or update tags/);
});
