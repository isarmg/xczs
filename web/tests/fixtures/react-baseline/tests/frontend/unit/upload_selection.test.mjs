import assert from "node:assert/strict";
import test from "node:test";

import {
  prepareUploadSelection,
} from "../../../web/dist/modules/upload/selection.js";
import { isValidAbsoluteLogicalPath, logicalChildPath } from "../../../web/dist/modules/shared/path.js";

test("upload selection enforces file-count and UTF-8 path budgets", () => {
  const OriginalFile = globalThis.File;
  class TestFile {
    constructor(name, relativePath = "") {
      this.name = name;
      this.webkitRelativePath = relativePath;
    }
  }
  globalThis.File = TestFile;
  try {
    const files = [
      new TestFile("a.txt"),
      new TestFile("b.txt"),
      new TestFile("c.txt"),
    ];
    const overCount = prepareUploadSelection(files, {
      fileLimit: 2,
      pathBytesLimit: 100,
    });
    assert.equal(overCount.ok, false);
    assert.equal(overCount.entries.length, 0);
    assert.match(overCount.error, /no more than 2 files/);

    const unicode = new TestFile("child.txt", "目录/child.txt");
    const expectedBytes = new TextEncoder().encode(unicode.webkitRelativePath)
      .byteLength;
    const accepted = prepareUploadSelection([unicode], {
      fileLimit: 1,
      pathBytesLimit: expectedBytes,
    });
    assert.equal(accepted.ok, true);
    assert.equal(accepted.totalPathBytes, expectedBytes);
    assert.equal(accepted.entries[0].name, unicode.webkitRelativePath);

    const overBytes = prepareUploadSelection([unicode], {
      fileLimit: 1,
      pathBytesLimit: expectedBytes - 1,
    });
    assert.equal(overBytes.ok, false);
    assert.equal(overBytes.entries.length, 0);
    assert.match(overBytes.error, /byte batch limit/);

    const invalidFiles = [
      new TestFile("bad\0name.txt"),
      new TestFile("文".repeat(86)),
      new TestFile("file.txt", `${"a".repeat(255)}/`.repeat(16) + "file.txt"),
    ];
    for (const invalid of invalidFiles) {
      const rejected = prepareUploadSelection([invalid]);
      assert.equal(rejected.ok, false, `path should be rejected: ${JSON.stringify(invalid)}`);
      assert.equal(rejected.entries.length, 0);
      assert.match(rejected.error, /not supported/);
    }
  } finally {
    if (OriginalFile === undefined) {
      delete globalThis.File;
    } else {
      globalThis.File = OriginalFile;
    }
  }
});

test("upload destination checks include the current directory bytes", () => {
  const fileName = "file.txt";
  const basePath = `/${Array(15).fill("a".repeat(255)).join("/")}`;
  assert.equal(isValidAbsoluteLogicalPath(logicalChildPath(basePath, fileName)), true);
  const tooLongBase = `${basePath}/${"b".repeat(255)}`;
  assert.equal(isValidAbsoluteLogicalPath(logicalChildPath(tooLongBase, fileName)), false);
});
