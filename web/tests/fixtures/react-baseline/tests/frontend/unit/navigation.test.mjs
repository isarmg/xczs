import assert from "node:assert/strict";
import test from "node:test";
import { navigationEntries, workspaceView } from "../../../web/react/navigation.ts";

test("current workspace deep links resolve without changing browser history", () => {
  for (const [hash, view] of [["", "files"], ["#", "files"], ["#files", "files"], ["#tags", "tags"], ["#status", "status"], ["#account", "account"]]) {
    assert.equal(workspaceView(hash), view);
  }
  assert.deepEqual(navigationEntries().map(entry => entry.href), ["#files", "#tags", "#status"]);
});

test("unknown workspace hashes are not treated as a successful menu selection", () => {
  for (const hash of ["#missing", "#tag-files", "#files/tags", "#Tags", "tags"]) {
    assert.equal(workspaceView(hash), "unknown");
  }
});
