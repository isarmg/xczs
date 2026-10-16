import assert from "node:assert/strict";
import test from "node:test";

import { classifyOperationResponse } from "../../../web/dist/modules/operations/protocol.js";

test("operation response protocol distinguishes success, failure, and uncertainty", () => {
  assert.deepEqual(classifyOperationResponse(204, "succeeded"), {
    kind: "success",
    outcomeUnknown: false,
  });
  assert.deepEqual(classifyOperationResponse(409, "rejected"), {
    kind: "error",
    outcomeUnknown: false,
  });
  assert.deepEqual(classifyOperationResponse(503, "unknown"), {
    kind: "error",
    outcomeUnknown: true,
  });
  for (const [status, state] of [
    [204, "running"],
    [204, "failed"],
    [409, "succeeded"],
    [409, "unexpected"],
  ]) {
    assert.deepEqual(classifyOperationResponse(status, state), {
      kind: "invalid",
      outcomeUnknown: true,
    });
  }
});
