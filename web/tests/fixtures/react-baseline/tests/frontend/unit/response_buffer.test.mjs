import assert from "node:assert/strict";
import test from "node:test";

import {
  ERROR_RESPONSE_BODY_LIMIT,
  SUCCESS_RESPONSE_BODY_LIMIT,
  bufferResponse,
} from "../../../web/dist/modules/http/client.js";

test("response buffering is byte bounded, cancels overflow, and remains replayable", async () => {
  const rejectedCode = async response => {
    try {
      await bufferResponse(response);
      return "accepted";
    } catch (error) {
      return error.code;
    }
  };

  let declaredCancelled = false;
  const declared = new Response(new ReadableStream({
    pull(controller) { controller.enqueue(new Uint8Array([1])); },
    cancel() { declaredCancelled = true; },
  }), {
    status: 500,
    headers: { "content-length": String(ERROR_RESPONSE_BODY_LIMIT + 1) },
  });
  assert.equal(await rejectedCode(declared), "response_body_too_large");
  await Promise.resolve();
  assert.equal(declaredCancelled, true);

  let streamedCancelled = false;
  let pulls = 0;
  const streamed = new Response(new ReadableStream({
    pull(controller) {
      pulls += 1;
      controller.enqueue(new Uint8Array(pulls === 1 ? ERROR_RESPONSE_BODY_LIMIT : 1));
    },
    cancel() { streamedCancelled = true; },
  }), { status: 500 });
  assert.equal(await rejectedCode(streamed), "response_body_too_large");
  await Promise.resolve();
  assert.equal(streamedCancelled, true);

  const payload = JSON.stringify({ message: "chunked ✓ response" });
  const bytes = new TextEncoder().encode(payload);
  const buffered = await bufferResponse(new Response(new ReadableStream({
    start(controller) {
      controller.enqueue(bytes.subarray(0, 7));
      controller.enqueue(bytes.subarray(7, bytes.length - 2));
      controller.enqueue(bytes.subarray(bytes.length - 2));
      controller.close();
    },
  }), { headers: { "content-type": "application/json" } }));
  const clone = buffered.clone();
  assert.deepEqual(await buffered.json(), { message: "chunked ✓ response" });
  assert.equal(await clone.text(), payload);

  let successCancelled = false;
  const oversizedSuccess = new Response(new ReadableStream({
    cancel() { successCancelled = true; },
  }), {
    headers: { "content-length": String(SUCCESS_RESPONSE_BODY_LIMIT + 1) },
  });
  assert.equal(await rejectedCode(oversizedSuccess), "response_body_too_large");
  await Promise.resolve();
  assert.equal(successCancelled, true);
});
