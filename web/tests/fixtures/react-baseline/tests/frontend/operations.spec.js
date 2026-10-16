const { randomUUID } = require("node:crypto");
const {
  actionDialog,
  chooseFileAction,
  currentDirectoryPath,
  currentLogicalChild,
  currentUrl,
  expect,
  pageData,
  rotateSession,
  rowByName,
  sameOriginRequestHeaders,
  selectFiles,
  submitActionDialog,
  test,
} = require("./fixtures");

const UUID_V4_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

function discardProtocolHeaders(uploadId, length, offset = length) {
  return {
    "X-Xczs-Upload-Id": uploadId,
    "X-Xczs-Upload-Length": String(length),
    "X-Xczs-Upload-Offset": String(offset),
    "X-Xczs-Operation-State": "rejected",
  };
}

function fulfillCreatedItemPreflight(route) {
  const { paths } = route.request().postDataJSON();
  return route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify({
      targets: paths.map(path => ({
        path,
        exists: true,
        revision: "d".repeat(64),
        replaceable: true,
      })),
    }),
  });
}

function fulfillTrackedFailure(route, status, code, detail, headers = {}) {
  const operationId = route.request().headers()["x-xczs-operation-id"];
  return route.fulfill({
    status,
    contentType: "application/problem+json",
    headers: {
      "X-Xczs-Operation-Id": operationId,
      "X-Xczs-Operation-State": "failed",
      ...headers,
    },
    body: JSON.stringify({
      type: `urn:xczs:problem:${code}`,
      title: "Precondition failed",
      status,
      detail,
      code,
    }),
  });
}

function fulfillDirectoryChanged(route) {
  return route.fulfill({
    status: 409,
    contentType: "application/problem+json",
    body: JSON.stringify({
      type: "urn:xczs:problem:directory_changed",
      title: "Directory changed",
      status: 409,
      detail: "Directory changed; restart listing",
      code: "directory_changed",
      recovery: "refresh_target",
    }),
  });
}

function inlineNameInput(page) {
  return page.locator(".inline-name-input");
}

async function fetchSameOriginRoute(route) {
  return route.fetch({
    headers: {
      ...(await route.request().allHeaders()),
      // route.fetch() is an APIRequestContext request, so Playwright does not
      // synthesize the Fetch Metadata header that the intercepted browser
      // request would have sent to the server.
      "Sec-Fetch-Site": "same-origin",
    },
  });
}

async function expectSelection(input, start, end) {
  expect(await input.evaluate(element => ({
    start: element.selectionStart,
    end: element.selectionEnd,
  }))).toEqual({ start, end });
}

async function seedFolder(page, name) {
  const data = await pageData(page);
  const root = currentDirectoryPath(page);
  const response = await page.context().request.post(
    new URL("/__xczs__/api/mkdir", page.url()).href,
    {
      headers: {
        ...sameOriginRequestHeaders(page),
        "Content-Type": "application/json",
        "X-CSRF-Token": data.session.csrf_token,
        "X-Xczs-Operation-Id": randomUUID(),
      },
      data: { path: `${root}/${name}` },
    },
  );
  expect(response.status()).toBe(201);
}

async function seedEmptyFile(page, name) {
  const data = await pageData(page);
  const response = await page.context().request.put(currentUrl(page, name), {
    headers: {
      ...sameOriginRequestHeaders(page),
      "X-CSRF-Token": data.session.csrf_token,
      "X-Xczs-Upload-Id": randomUUID(),
      "X-Xczs-Upload-Length": "0",
      "X-Xczs-Upload-Overwrite": "false",
    },
    data: Buffer.alloc(0),
  });
  expect(response.status()).toBe(201);
}

test("Immediate empty-file creation neither queries nor replays PUT after 429 not-started", async ({
  appPage: page,
}) => {
  let putCount = 0;
  let headCount = 0;
  await page.route("**/newfile", async route => {
    const request = route.request();
    if (request.method() === "HEAD") {
      headCount++;
      await route.abort();
      return;
    }
    putCount++;
    const uploadId = request.headers()["x-xczs-upload-id"];
    await route.fulfill({
      status: 429,
      contentType: "application/problem+json",
      headers: {
        "X-Xczs-Upload-Id": uploadId,
        "X-Xczs-Upload-Length": "0",
        "X-Xczs-Operation-State": "not-started",
      },
      body: JSON.stringify({
        type: "urn:xczs:problem:upload_concurrency_limit",
        title: "Too Many Requests",
        status: 429,
        detail: "Too many concurrent uploads",
        code: "upload_concurrency_limit",
        recovery: "retry",
        upload_id: uploadId,
        upload_state: "not-started",
        upload_length: 0,
      }),
    });
  });
  await page.getByRole("button", { name: "New empty file" }).click();
  const errorDialog = actionDialog(page, "Create file failed");
  await expect(errorDialog).toContainText("Too many concurrent uploads");
  expect(putCount).toBe(1);
  expect(headCount).toBe(0);
  await expect(page.getByRole("button", {
    name: "Refresh",
    exact: true,
  })).toHaveCount(0);
  await errorDialog.getByRole("button", { name: "Close" }).click();
});

test("Unexpected 2xx on immediate empty-file creation permits one HEAD confirmation with the same upload ID", async ({
  appPage: page,
}) => {
  let putCount = 0;
  let headCount = 0;
  let uploadId = "";
  let documentRequests = 0;
  page.on("request", request => {
    if (request.resourceType() === "document") documentRequests++;
  });
  await page.route(
    "**/__xczs__/api/upload/preflight",
    fulfillCreatedItemPreflight,
  );
  await page.route("**/newfile", async route => {
    const request = route.request();
    if (request.method() === "HEAD") {
      headCount++;
      expect(request.headers()["x-xczs-upload-id"]).toBe(uploadId);
      await route.fulfill({
        status: 200,
        headers: {
          "X-Xczs-Upload-Id": uploadId,
          "X-Xczs-Upload-Length": "0",
          "X-Xczs-Upload-Offset": "0",
          "X-Xczs-Operation-State": "committed",
        },
        body: "",
      });
      return;
    }
    putCount++;
    uploadId = request.headers()["x-xczs-upload-id"];
    await route.fulfill({
      status: 202,
      headers: {
        "X-Xczs-Upload-Id": uploadId,
        "X-Xczs-Upload-Length": "0",
        "X-Xczs-Upload-Offset": "0",
        "X-Xczs-Operation-State": "committed",
      },
      body: "",
    });
  });
  await page.getByRole("button", { name: "New empty file" }).click();
  await expect.poll(() => headCount).toBe(1);
  expect(putCount).toBe(1);
  expect(headCount).toBe(1);
  expect(documentRequests).toBe(0);
  await expect(inlineNameInput(page)).toHaveValue("newfile");
  await expect(inlineNameInput(page)).toBeFocused();
  await expect(actionDialog(page, "Create file failed")).toBeHidden();
});

test("New folders immediately create newfolder and enter inline editing", async ({
  appPage: page,
}) => {
  const root = currentDirectoryPath(page);
  const directoryUrl = page.url();
  const responsePromise = page.waitForResponse(
    response =>
      response.request().method() === "POST" &&
      new URL(response.url()).pathname.endsWith("/__xczs__/api/mkdir"),
  );
  await page.getByRole("button", { name: "New folder" }).click();
  const response = await responsePromise;
  expect(response.status()).toBe(201);
  expect(response.request().headers()["x-xczs-operation-id"])
    .toMatch(UUID_V4_PATTERN);
  expect(response.request().postDataJSON()).toEqual({
    path: `${root}/newfolder`,
  });
  await expect(page).toHaveURL(directoryUrl);
  await expect(actionDialog(page, "Create folder")).toBeHidden();
  const input = inlineNameInput(page);
  await expect(input).toHaveCount(1);
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("newfolder");
  await expectSelection(input, "newfolder".length, "newfolder".length);
  const renamePromise = page.waitForResponse(
    candidate =>
      candidate.request().method() === "POST" &&
      new URL(candidate.url()).pathname.endsWith("/__xczs__/api/rename"),
  );
  await input.fill("created-by-browser");
  await input.press("Enter");
  const renameResponse = await renamePromise;
  expect(renameResponse.status()).toBe(204);
  expect(renameResponse.request().postDataJSON()).toEqual({
    source: `${root}/newfolder`,
    name: "created-by-browser",
    source_revision: expect.stringMatching(/^[0-9a-f]{64}$/),
    overwrite: false,
  });
  await expect(input).toHaveCount(0);
  await expect(rowByName(page, "created-by-browser")).toBeVisible();
});

test("New empty files immediately PUT newfile and enter inline editing", async ({
  appPage: page,
}) => {
  const responsePromise = page.waitForResponse(
    response =>
      response.request().method() === "PUT" &&
      new URL(response.url()).pathname.endsWith("/newfile"),
  );
  await page.getByRole("button", { name: "New empty file" }).click();
  const response = await responsePromise;
  expect(response.status()).toBe(201);
  expect(response.request().headers()["x-xczs-upload-length"]).toBe("0");
  expect(response.request().headers()["x-xczs-upload-id"])
    .toMatch(UUID_V4_PATTERN);
  expect(response.request().headers()["x-xczs-upload-overwrite"]).toBe("false");
  await expect(actionDialog(page, "Create empty file")).toBeHidden();
  const input = inlineNameInput(page);
  await expect(input).toHaveCount(1);
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("newfile");
  await expectSelection(input, "newfile".length, "newfile".length);
  await input.press("Escape");
  await expect(input).toHaveCount(0);
  await expect(rowByName(page, "newfile")).toBeVisible();
});

test("Completed creation clears an older row editor started concurrently", async ({ appPage: page }) => {
  let releasePreflight;
  let markPreflightStarted;
  const preflightGate = new Promise(resolve => {
    releasePreflight = resolve;
  });
  const preflightStarted = new Promise(resolve => {
    markPreflightStarted = resolve;
  });
  await page.route("**/__xczs__/api/upload/preflight", async route => {
    markPreflightStarted();
    await preflightGate;
    await fulfillCreatedItemPreflight(route);
  });

  await page.getByRole("button", { name: "New empty file" }).click();
  await preflightStarted;
  await chooseFileAction(page, "rename", "rename-me.txt");
  await expect(inlineNameInput(page)).toHaveValue("rename-me.txt");

  releasePreflight();
  await expect(inlineNameInput(page)).toHaveCount(1);
  await expect(inlineNameInput(page)).toHaveValue("newfile");
  await expect(inlineNameInput(page)).toBeFocused();
  await expect(rowByName(page, "rename-me.txt")).toBeVisible();
  await expect(page.locator(".paths-table tbody tr.is-renaming")).toHaveCount(1);
});

for (const [label, createdName] of [
  ["New folder", "newfolder"],
  ["New empty file", "newfile"],
]) {
  test(`${label} invalidates the listing immediately after submission without waiting for the revision query`, async ({
    appPage: page,
  }) => {
    let releasePreflight;
    let markPreflightStarted;
    const preflightGate = new Promise(resolve => {
      releasePreflight = resolve;
    });
    const preflightStarted = new Promise(resolve => {
      markPreflightStarted = resolve;
    });
    await page.route("**/__xczs__/api/upload/preflight", async route => {
      markPreflightStarted();
      await preflightGate;
      await fulfillCreatedItemPreflight(route);
    });

    await page.getByRole("button", { name: label, exact: true }).click();
    await preflightStarted;

    const refresh = page.getByRole("button", { name: "Refresh", exact: true });
    await expect(refresh).toBeVisible();
    await expect(page.locator(".list-status")).toContainText(
      "Folder contents changed",
    );

    releasePreflight();
    await expect(inlineNameInput(page)).toHaveValue(createdName);
  });
}

test("Default folder names increment only after trusted conflicts and each candidate uses a new operation ID", async ({
  appPage: page,
}) => {
  await seedFolder(page, "newfolder");
  await seedFolder(page, "newfolder (2)");
  const requests = [];
  page.on("request", request => {
    if (
      request.method() === "POST" &&
      new URL(request.url()).pathname.endsWith("/__xczs__/api/mkdir")
    ) {
      requests.push({
        body: request.postDataJSON(),
        operationId: request.headers()["x-xczs-operation-id"],
      });
    }
  });

  await page.getByRole("button", { name: "New folder" }).click();
  const input = inlineNameInput(page);
  await expect(input).toHaveValue("newfolder (3)");
  await expect(input).toBeFocused();

  const root = currentDirectoryPath(page);
  expect(requests.map(request => request.body)).toEqual([
    { path: `${root}/newfolder` },
    { path: `${root}/newfolder (2)` },
    { path: `${root}/newfolder (3)` },
  ]);
  expect(requests.map(request => request.operationId))
    .toEqual(requests.map(request => expect.stringMatching(UUID_V4_PATTERN)));
  expect(new Set(requests.map(request => request.operationId)).size).toBe(3);
});

test("A job error with path_exists must not increment the default folder name", async ({
  appPage: page,
}) => {
  const requests = [];
  let statusQueries = 0;
  await page.route("**/__xczs__/api/mkdir", route => {
    requests.push(route.request().postDataJSON());
    if (requests.length > 1) {
      return fulfillTrackedFailure(
        route,
        500,
        "unexpected_second_attempt",
        "A second candidate must not be attempted",
      );
    }
    const operationId = route.request().headers()["x-xczs-operation-id"];
    return route.fulfill({
      status: 500,
      contentType: "application/problem+json",
      headers: {
        "X-Xczs-Operation-Id": operationId,
        "X-Xczs-Operation-State": "unknown",
      },
      body: JSON.stringify({
        type: "urn:xczs:problem:outcome_uncertain",
        title: "Internal server error",
        status: 500,
        code: "outcome_uncertain",
        detail: "The create result must be reconciled",
        recovery: "query_job",
      }),
    });
  });
  await page.route("**/__xczs__/api/jobs/*", route => {
    statusQueries++;
    const operationId = new URL(route.request().url()).pathname.split("/").pop();
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      headers: {
        "X-Xczs-Operation-Id": operationId,
        "X-Xczs-Operation-State": "failed",
      },
      body: JSON.stringify({
        job_id: operationId,
        state: "failed",
        http_status: 500,
        code: "path_exists",
        detail: "The server failed without proving a name conflict",
      }),
    });
  });

  await page.getByRole("button", { name: "New folder" }).click();
  await expect(actionDialog(page, "Create folder failed")).toBeVisible();
  expect(requests).toEqual([
    { path: `${currentDirectoryPath(page)}/newfolder` },
  ]);
  expect(statusQueries).toBe(1);
  await expect(inlineNameInput(page)).toHaveCount(0);
});

test("Both creation actions share one pending state so rapid clicks neither create concurrently nor replace the editor", async ({
  appPage: page,
}) => {
  let releaseFolder;
  let markFolderStarted;
  let filePuts = 0;
  const folderGate = new Promise(resolve => {
    releaseFolder = resolve;
  });
  const folderStarted = new Promise(resolve => {
    markFolderStarted = resolve;
  });
  await page.route("**/__xczs__/api/mkdir", async route => {
    markFolderStarted();
    await folderGate;
    await route.continue();
  });
  page.on("request", request => {
    if (request.method() === "PUT") filePuts++;
  });

  await page.getByRole("button", { name: "New folder" }).click();
  await folderStarted;
  await expect(page.locator(".operation-status")).toHaveText(
    "Creating a new folder…",
  );
  await page.getByRole("button", { name: "New empty file" }).click();
  await page.evaluate(() => new Promise(resolve => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
  expect(filePuts).toBe(0);
  await expect(page.locator(".operation-status")).toHaveText(
    "Creating a new folder…",
  );

  releaseFolder();
  await expect(inlineNameInput(page)).toHaveValue("newfolder");
  await expect(inlineNameInput(page)).toHaveCount(1);
  await expect(inlineNameInput(page)).toBeFocused();
  expect(filePuts).toBe(0);
});

test("An existing empty-file target increments immediately after a not-started conflict without discard", async ({
  appPage: page,
}) => {
  await seedEmptyFile(page, "newfile");
  await seedEmptyFile(page, "newfile (2)");
  const puts = [];
  const discards = [];
  page.on("request", request => {
    const url = new URL(request.url());
    if (request.method() === "PUT") {
      puts.push({
        path: decodeURIComponent(url.pathname),
        uploadId: request.headers()["x-xczs-upload-id"],
        overwrite: request.headers()["x-xczs-upload-overwrite"],
      });
    } else if (
      request.method() === "POST" &&
      url.pathname.endsWith("/__xczs__/api/upload/discard")
    ) {
      discards.push(request.postDataJSON());
    }
  });

  await page.getByRole("button", { name: "New empty file" }).click();
  await expect(inlineNameInput(page)).toHaveValue("newfile (3)");

  const root = currentDirectoryPath(page);
  expect(puts.map(request => request.path)).toEqual([
    `${root}/newfile`,
    `${root}/newfile (2)`,
    `${root}/newfile (3)`,
  ]);
  expect(puts.every(request => request.overwrite === "false")).toBe(true);
  expect(puts.map(request => request.uploadId))
    .toEqual(puts.map(request => expect.stringMatching(UUID_V4_PATTERN)));
  expect(new Set(puts.map(request => request.uploadId)).size).toBe(3);
  expect(discards).toEqual([]);
});

test("An empty-file awaiting-confirmation conflict increments only after successful discard", async ({
  appPage: page,
}) => {
  const puts = [];
  const discards = [];
  await page.route(
    "**/__xczs__/api/upload/preflight",
    fulfillCreatedItemPreflight,
  );
  await page.route("**/newfile*", route => {
    const request = route.request();
    if (request.method() !== "PUT") return route.continue();
    const uploadId = request.headers()["x-xczs-upload-id"];
    puts.push({
      path: decodeURIComponent(new URL(request.url()).pathname),
      uploadId,
    });
    if (puts.length === 1) {
      return route.fulfill({
        status: 409,
        contentType: "application/problem+json",
        headers: {
          "X-Xczs-Upload-Id": uploadId,
          "X-Xczs-Upload-Length": "0",
          "X-Xczs-Upload-Offset": "0",
          "X-Xczs-Operation-State": "awaiting-confirmation",
          "X-Xczs-Target-Revision": "a".repeat(64),
          "X-Xczs-Target-Replaceable": "true",
        },
        body: JSON.stringify({
          type: "urn:xczs:problem:destination_exists",
          title: "Conflict",
          status: 409,
          code: "destination_exists",
          detail: "Destination exists after staging",
          recovery: "refresh_target",
        }),
      });
    }
    return route.fulfill({
      status: 201,
      headers: {
        "X-Xczs-Upload-Id": uploadId,
        "X-Xczs-Upload-Length": "0",
        "X-Xczs-Upload-Offset": "0",
        "X-Xczs-Operation-State": "committed",
      },
      body: "",
    });
  });
  await page.route("**/__xczs__/api/upload/discard", route => {
    const payload = route.request().postDataJSON();
    discards.push(payload);
    return route.fulfill({
      status: 204,
      headers: discardProtocolHeaders(payload.upload_id, 0),
      body: "",
    });
  });

  await page.getByRole("button", { name: "New empty file" }).click();
  await expect(inlineNameInput(page)).toHaveValue("newfile (2)");
  const root = currentDirectoryPath(page);
  expect(puts.map(request => request.path)).toEqual([
    `${root}/newfile`,
    `${root}/newfile (2)`,
  ]);
  expect(puts[0].uploadId).toMatch(UUID_V4_PATTERN);
  expect(puts[1].uploadId).toMatch(UUID_V4_PATTERN);
  expect(puts[1].uploadId).not.toBe(puts[0].uploadId);
  expect(discards).toEqual([{
    path: `${root}/newfile`,
    upload_id: puts[0].uploadId,
  }]);
});

test("An empty-file rejected conflict tries the next candidate with a new ID without discard", async ({
  appPage: page,
}) => {
  const puts = [];
  let discards = 0;
  await page.route(
    "**/__xczs__/api/upload/preflight",
    fulfillCreatedItemPreflight,
  );
  await page.route("**/newfile*", route => {
    const request = route.request();
    if (request.method() !== "PUT") return route.continue();
    const uploadId = request.headers()["x-xczs-upload-id"];
    puts.push({
      path: decodeURIComponent(new URL(request.url()).pathname),
      uploadId,
    });
    if (puts.length === 1) {
      return route.fulfill({
        status: 409,
        contentType: "application/problem+json",
        headers: {
          "X-Xczs-Upload-Id": uploadId,
          "X-Xczs-Upload-Length": "0",
          "X-Xczs-Operation-State": "rejected",
        },
        body: JSON.stringify({
          type: "urn:xczs:problem:destination_exists",
          title: "Conflict",
          status: 409,
          code: "destination_exists",
          detail: "Destination exists",
        }),
      });
    }
    return route.fulfill({
      status: 201,
      headers: {
        "X-Xczs-Upload-Id": uploadId,
        "X-Xczs-Upload-Length": "0",
        "X-Xczs-Upload-Offset": "0",
        "X-Xczs-Operation-State": "committed",
      },
      body: "",
    });
  });
  await page.route("**/__xczs__/api/upload/discard", route => {
    discards++;
    const payload = route.request().postDataJSON();
    return route.fulfill({
      status: 204,
      headers: discardProtocolHeaders(payload.upload_id, 0),
      body: "",
    });
  });

  await page.getByRole("button", { name: "New empty file" }).click();
  await expect(inlineNameInput(page)).toHaveValue("newfile (2)");
  expect(puts.map(request => request.path)).toEqual([
    `${currentDirectoryPath(page)}/newfile`,
    `${currentDirectoryPath(page)}/newfile (2)`,
  ]);
  expect(puts[0].uploadId).toMatch(UUID_V4_PATTERN);
  expect(puts[1].uploadId).toMatch(UUID_V4_PATTERN);
  expect(puts[1].uploadId).not.toBe(puts[0].uploadId);
  expect(discards).toBe(0);
});

test("HTTP 500 destination_exists must not increment the default empty-file name", async ({
  appPage: page,
}) => {
  const puts = [];
  let discards = 0;
  await page.route("**/newfile*", route => {
    const request = route.request();
    if (request.method() !== "PUT") return route.continue();
    const uploadId = request.headers()["x-xczs-upload-id"];
    puts.push(decodeURIComponent(new URL(request.url()).pathname));
    const firstAttempt = puts.length === 1;
    const status = firstAttempt ? 500 : 507;
    const code = firstAttempt
      ? "destination_exists"
      : "unexpected_second_attempt";
    return route.fulfill({
      status,
      contentType: "application/problem+json",
      headers: {
        "X-Xczs-Upload-Id": uploadId,
        "X-Xczs-Upload-Length": "0",
        "X-Xczs-Operation-State": "rejected",
      },
      body: JSON.stringify({
        type: `urn:xczs:problem:${code}`,
        title: "Upload rejected",
        status,
        code,
        detail: firstAttempt
          ? "The server failed without proving a destination conflict"
          : "A second candidate must not be attempted",
      }),
    });
  });
  await page.route("**/__xczs__/api/upload/discard", route => {
    discards++;
    return route.fulfill({ status: 500, body: "unexpected discard" });
  });

  await page.getByRole("button", { name: "New empty file" }).click();
  await expect(actionDialog(page, "Create file failed")).toBeVisible();
  expect(puts).toEqual([`${currentDirectoryPath(page)}/newfile`]);
  expect(discards).toBe(0);
  await expect(inlineNameInput(page)).toHaveCount(0);
});

test("Unknown folder creation outcomes must not try the next candidate name", async ({
  appPage: page,
}) => {
  const requests = [];
  let statusQueries = 0;
  await page.route("**/__xczs__/api/mkdir", route => {
    const operationId = route.request().headers()["x-xczs-operation-id"];
    requests.push(route.request().postDataJSON());
    return route.fulfill({
      status: 409,
      contentType: "application/problem+json",
      headers: {
        "X-Xczs-Operation-Id": operationId,
        "X-Xczs-Operation-State": "unknown",
      },
      body: JSON.stringify({
        type: "urn:xczs:problem:path_exists",
        title: "Conflict",
        status: 409,
        code: "path_exists",
        detail: "The create result is unknown",
        recovery: "query_job",
      }),
    });
  });
  await page.route("**/__xczs__/api/jobs/*", route => {
    statusQueries++;
    const operationId = new URL(route.request().url()).pathname.split("/").pop();
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      headers: {
        "X-Xczs-Operation-Id": operationId,
        "X-Xczs-Operation-State": "unknown",
      },
      body: JSON.stringify({
        job_id: operationId,
        state: "unknown",
        http_status: 409,
        code: "outcome_uncertain",
        detail: "The create result is unknown",
      }),
    });
  });

  await page.getByRole("button", { name: "New folder" }).click();
  await expect(actionDialog(page, "Create folder failed")).toBeVisible();
  expect(requests).toEqual([
    { path: `${currentDirectoryPath(page)}/newfolder` },
  ]);
  expect(statusQueries).toBe(1);
  await expect(inlineNameInput(page)).toHaveCount(0);
  await expect(page.getByRole("button", {
    name: "Refresh",
    exact: true,
  })).toBeVisible();
});

test("Untrusted discard results prevent PUT of the next empty-file candidate", async ({
  appPage: page,
}) => {
  const puts = [];
  let discards = 0;
  await page.route("**/newfile", route => {
    const request = route.request();
    const uploadId = request.headers()["x-xczs-upload-id"];
    puts.push({ path: new URL(request.url()).pathname, uploadId });
    return route.fulfill({
      status: 409,
      contentType: "application/problem+json",
      headers: {
        "X-Xczs-Upload-Id": uploadId,
        "X-Xczs-Upload-Length": "0",
        "X-Xczs-Upload-Offset": "0",
        "X-Xczs-Operation-State": "awaiting-confirmation",
        "X-Xczs-Target-Revision": "b".repeat(64),
        "X-Xczs-Target-Replaceable": "true",
      },
      body: JSON.stringify({
        type: "urn:xczs:problem:destination_exists",
        title: "Conflict",
        status: 409,
        code: "destination_exists",
        detail: "Destination exists",
        recovery: "refresh_target",
      }),
    });
  });
  await page.route("**/__xczs__/api/upload/discard", route => {
    discards++;
    return route.fulfill({ status: 502, body: "discard outcome unknown" });
  });

  await page.getByRole("button", { name: "New empty file" }).click();
  await expect(actionDialog(page, "Create file failed")).toBeVisible();
  expect(puts).toHaveLength(1);
  expect(discards).toBe(1);
  await expect(inlineNameInput(page)).toHaveCount(0);
  await expect(page.getByRole("button", {
    name: "Refresh",
    exact: true,
  })).toBeVisible();
  await expect(page.locator(".list-status")).toContainText(
    "may have changed",
  );
});

test("Renaming uses the dedicated endpoint and refreshes the current directory", async ({ appPage: page }) => {
  const source = currentLogicalChild(page, "rename-me.txt");
  const directoryUrl = page.url();
  const responsePromise = page.waitForResponse(
    response =>
      response.request().method() === "POST" &&
      new URL(response.url()).pathname.endsWith("/__xczs__/api/rename"),
  );
  await chooseFileAction(page, "rename", "rename-me.txt");
  const input = inlineNameInput(page);
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("rename-me.txt");
  await expect(input.locator("xpath=ancestor::td")).toHaveClass(/\bcell-name\b/);
  await expect(input.locator("xpath=ancestor::tr")).toHaveClass(/\bis-renaming\b/);
  await expect(actionDialog(page, "Rename item")).toBeHidden();
  await expectSelection(input, "rename-me".length, "rename-me".length);
  await input.fill("renamed-by-browser.txt");
  await input.press("Enter");
  const response = await responsePromise;
  expect(response.status()).toBe(204);
  expect(response.request().headers()["x-xczs-operation-id"]).toMatch(
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
  );
  expect(response.request().postDataJSON()).toEqual({
    source,
    name: "renamed-by-browser.txt",
    source_revision: expect.stringMatching(/^[0-9a-f]{64}$/),
    overwrite: false,
  });
  await expect(page).toHaveURL(directoryUrl);
  await expect(
    page.getByRole("link", { name: "renamed-by-browser.txt", exact: true }),
  ).toBeVisible();
  await expect(rowByName(page, "rename-me.txt")).toHaveCount(0);
});

test("Synchronous blur during a slow busy rename must not submit twice", async ({
  appPage: page,
}) => {
  let releaseRename;
  let markRenameStarted;
  let renameRequests = 0;
  const renameGate = new Promise(resolve => {
    releaseRename = resolve;
  });
  const renameStarted = new Promise(resolve => {
    markRenameStarted = resolve;
  });
  await page.route("**/__xczs__/api/rename", async route => {
    renameRequests++;
    markRenameStarted();
    await renameGate;
    await route.continue();
  });

  await chooseFileAction(page, "rename", "rename-me.txt");
  const input = inlineNameInput(page);
  await input.fill("slow-renamed.txt");
  await input.press("Enter");
  await renameStarted;
  await expect(input).toBeDisabled();
  await expect(input).toHaveAttribute("aria-busy", "true");
  await page.evaluate(() => new Promise(resolve => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
  expect(renameRequests).toBe(1);

  releaseRename();
  const renamedLink = page.getByRole("link", {
    name: "slow-renamed.txt",
    exact: true,
  });
  await expect(renamedLink).toBeVisible();
  await expect(input).toHaveCount(0);
  await expect(renamedLink).toBeFocused();
  expect(renameRequests).toBe(1);
});

test("Inline rename supports Tab, blur submission, and Escape cancellation with only one editor globally", async ({
  appPage: page,
}) => {
  const requests = [];
  page.on("request", request => {
    if (
      request.method() === "POST" &&
      new URL(request.url()).pathname.endsWith("/__xczs__/api/rename")
    ) {
      requests.push(request.postDataJSON());
    }
  });

  await chooseFileAction(page, "rename", "rename-me.txt");
  const firstInput = inlineNameInput(page);
  await expect(firstInput).toBeFocused();
  await expectSelection(firstInput, "rename-me".length, "rename-me".length);

  await chooseFileAction(page, "rename", "download-me.txt");
  await expect(inlineNameInput(page)).toHaveCount(1);
  await expect(inlineNameInput(page)).toHaveValue("download-me.txt");
  await expect(inlineNameInput(page)).toBeFocused();
  expect(requests).toHaveLength(0);

  const nextName = await page.locator(".is-renaming")
    .locator("xpath=following-sibling::tr[1]").locator(".cell-name a").innerText();
  await inlineNameInput(page).fill("tab-renamed.txt");
  await inlineNameInput(page).press("Tab");
  await expect(rowByName(page, "tab-renamed.txt")).toBeVisible();
  await expect(rowByName(page, nextName).locator(".cell-name a")).toBeFocused();
  await expect(inlineNameInput(page)).toHaveCount(0);

  const escapeRename = page.locator('[data-file-action="rename"]');
  await chooseFileAction(page, "rename", "rename-me.txt");
  await inlineNameInput(page).fill("must-not-commit.txt");
  await inlineNameInput(page).press("Escape");
  await expect(rowByName(page, "rename-me.txt")).toBeVisible();
  await expect(escapeRename).toBeFocused();

  await chooseFileAction(page, "rename", "rename-me.txt");
  await inlineNameInput(page).fill("blur-renamed.txt");
  await page.getByRole("button", { name: "New folder" }).focus();
  await expect(rowByName(page, "blur-renamed.txt")).toBeVisible();
  expect(requests.map(request => request.name)).toEqual([
    "tab-renamed.txt",
    "blur-renamed.txt",
  ]);
});

test("Entering inline editing avoids automatic selection and places the caret at the name editing position", async ({
  appPage: page,
}) => {
  await chooseFileAction(page, "rename", "existing-folder");
  await expectSelection(
    inlineNameInput(page),
    "existing-folder".length,
    "existing-folder".length,
  );
  await inlineNameInput(page).press("Escape");

  await chooseFileAction(page, "rename", "special & # + 中文.txt");
  await expectSelection(
    inlineNameInput(page),
    "special & # + 中文".length,
    "special & # + 中文".length,
  );
  await inlineNameInput(page).press("Escape");
});

test("Escape during IME composition preserves editing while ordinary Escape cancels it", async ({
  appPage: page,
}) => {
  let renameRequests = 0;
  page.on("request", request => {
    if (
      request.method() === "POST" &&
      new URL(request.url()).pathname.endsWith("/__xczs__/api/rename")
    ) {
      renameRequests++;
    }
  });
  await chooseFileAction(page, "rename", "rename-me.txt");
  const input = inlineNameInput(page);
  await input.fill("组合输入.txt");
  await input.dispatchEvent("keydown", {
    key: "Escape",
    bubbles: true,
    isComposing: true,
  });
  await expect(input).toHaveValue("组合输入.txt");
  await expect(input).toBeFocused();
  expect(renameRequests).toBe(0);

  await input.press("Escape");
  await expect(input).toHaveCount(0);
  await expect(rowByName(page, "rename-me.txt")).toBeVisible();
  expect(renameRequests).toBe(0);
});

test("Search and sort states still rename inline before refreshing through the active filters", async ({
  appPage: page,
}) => {
  const url = new URL(page.url());
  url.searchParams.set("q", "does-not-match-default-name");
  url.searchParams.set("sort", "size");
  url.searchParams.set("order", "desc");
  await page.goto(url.href);
  const trigger = page.getByRole("button", { name: "New folder" });
  await trigger.click();

  const input = inlineNameInput(page);
  await expect(input).toHaveValue("newfolder");
  await expect(input).toBeFocused();
  expect(new URL(page.url()).searchParams.get("q"))
    .toBe("does-not-match-default-name");
  expect(new URL(page.url()).searchParams.get("sort")).toBe("size");
  expect(new URL(page.url()).searchParams.get("order")).toBe("desc");

  await input.press("Escape");
  await expect(input).toHaveCount(0);
  await expect(page.getByRole("link", {
    name: "newfolder",
    exact: true,
  })).toHaveCount(0);
  await expect(page.locator(".list-status")).toContainText(/created.*hidden/i);
  await expect(trigger).toBeFocused();
});

test("Moving uses the dedicated endpoint, preserves the name, and enters the destination directory", async ({
  appPage: page,
}) => {
  const source = currentLogicalChild(page, "rename-me.txt");
  const directory = currentLogicalChild(page, "existing-folder");
  const responsePromise = page.waitForResponse(
    response =>
      response.request().method() === "POST" &&
      new URL(response.url()).pathname.endsWith("/__xczs__/api/move"),
  );
  await chooseFileAction(page, "move", "rename-me.txt");
  await submitActionDialog(page, {
    title: "Move item",
    label: "Destination folder",
    value: directory,
    confirmText: "Move",
  });
  const response = await responsePromise;
  expect(response.status()).toBe(204);
  expect(response.request().headers()["x-xczs-operation-id"]).toMatch(
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
  );
  expect(response.request().postDataJSON()).toEqual({
    source,
    directory,
    source_revision: expect.stringMatching(/^[0-9a-f]{64}$/),
    overwrite: false,
  });
  await expect.poll(() => currentDirectoryPath(page)).toBe(directory);
  await expect(
    page.getByRole("link", { name: "rename-me.txt", exact: true }),
  ).toBeVisible();
});

test("Enter in the move destination prompt confirms a Unicode folder path", async ({
  appPage: page,
}) => {
  const folderName = "会议资料 & 计划";
  await seedFolder(page, folderName);
  const directory = currentLogicalChild(page, folderName);
  await chooseFileAction(page, "move", "rename-me.txt");
  const dialog = actionDialog(page, "Move item");
  const input = dialog.getByRole("textbox", { name: "Destination folder" });
  await input.fill(directory);
  const responsePromise = page.waitForResponse(response =>
    response.request().method() === "POST" &&
    new URL(response.url()).pathname.endsWith("/__xczs__/api/move"),
  );
  await input.press("Enter");
  const response = await responsePromise;
  expect(response.status()).toBe(204);
  expect(response.request().postDataJSON().directory).toBe(directory);
  await expect.poll(() => currentDirectoryPath(page)).toBe(directory);
  await expect(page.getByRole("link", {
    name: "rename-me.txt",
    exact: true,
  })).toBeVisible();
});

test("An existing rename target permits explicit overwrite only after confirmation", async ({
  appPage: page,
  context,
}) => {
  const source = currentLogicalChild(page, "overwrite-source.txt");
  const destination = currentLogicalChild(page, "overwrite-target.txt");
  const targetUrl = currentUrl(page, "overwrite-target.txt");
  let targetRevision = "";
  await page.route("**/__xczs__/api/rename", async route => {
    const request = route.request().postDataJSON();
    if (
      request.source === source &&
      request.name === "overwrite-target.txt" &&
      request.overwrite === false
    ) {
      const response = await fetchSameOriginRoute(route);
      targetRevision = response.headers()["x-xczs-target-revision"] || "";
      await route.fulfill({ response });
      return;
    }
    await route.continue();
  });
  const responses = [];
  page.on("response", response => {
    if (
      response.request().method() === "POST" &&
      new URL(response.url()).pathname.endsWith("/__xczs__/api/rename")
    ) {
      responses.push(response);
    }
  });
  await chooseFileAction(page, "rename", "overwrite-source.txt");
  const input = inlineNameInput(page);
  await input.fill("overwrite-target.txt");
  await input.press("Enter");
  const overwriteDialog = actionDialog(page, "Overwrite destination?");
  await expect(overwriteDialog).toContainText(
    `Replace "${destination}" with "${source}"?`,
  );
  await overwriteDialog.getByRole("button", { name: "Overwrite" }).click();
  await expect.poll(() => responses.length).toBe(2);
  expect(responses.map(response => response.status())).toEqual([409, 204]);
  expect(targetRevision).toMatch(/^[0-9a-f]{64}$/);
  const sourceRevision = responses[0].request().postDataJSON().source_revision;
  expect(sourceRevision).toMatch(/^[0-9a-f]{64}$/);
  expect(responses.map(response => response.request().postDataJSON())).toEqual([
    {
      source,
      name: "overwrite-target.txt",
      source_revision: sourceRevision,
      overwrite: false,
    },
    {
      source,
      name: "overwrite-target.txt",
      source_revision: sourceRevision,
      overwrite: true,
      destination_revision: targetRevision,
    },
  ]);
  const contentResponse = await context.request.get(targetUrl);
  expect(contentResponse.status()).toBe(200);
  expect(await contentResponse.text()).toBe("replacement content");
});

test("Rename conflicts without a destination revision reject overwrite confirmation", async ({
  appPage: page,
}) => {
  let renameRequests = 0;
  await page.route("**/__xczs__/api/rename", route => {
    renameRequests++;
    const operationId = route.request().headers()["x-xczs-operation-id"];
    return route.fulfill({
      status: 409,
      contentType: "application/problem+json",
      headers: {
        "X-Xczs-Operation-Id": operationId,
        "X-Xczs-Operation-State": "failed",
      },
      body: JSON.stringify({
        type: "urn:xczs:problem:destination_exists",
        title: "Conflict",
        status: 409,
        detail: "The target name is occupied",
        code: "destination_exists",
      }),
    });
  });

  await chooseFileAction(page, "rename", "overwrite-source.txt");
  const input = inlineNameInput(page);
  await input.fill("overwrite-target.txt");
  await input.press("Enter");

  await expect(actionDialog(page, "Rename failed")).toContainText(
    "The target name is occupied",
  );
  await expect(actionDialog(page, "Overwrite destination?")).toBeHidden();
  expect(renameRequests).toBe(1);
});

test("HTTP 500 destination_exists must not authorize rename overwrite", async ({
  appPage: page,
}) => {
  let renameRequests = 0;
  await page.route("**/__xczs__/api/rename", route => {
    renameRequests++;
    const operationId = route.request().headers()["x-xczs-operation-id"];
    return route.fulfill({
      status: 500,
      contentType: "application/problem+json",
      headers: {
        "X-Xczs-Operation-Id": operationId,
        "X-Xczs-Operation-State": "failed",
        "X-Xczs-Target-Revision": "a".repeat(64),
      },
      body: JSON.stringify({
        type: "urn:xczs:problem:destination_exists",
        title: "Internal server error",
        status: 500,
        detail: "The server failed without proving a destination conflict",
        code: "destination_exists",
      }),
    });
  });

  await chooseFileAction(page, "rename", "overwrite-source.txt");
  const input = inlineNameInput(page);
  await input.fill("overwrite-target.txt");
  await input.press("Enter");

  await expect(actionDialog(page, "Rename failed")).toContainText(
    "The server failed without proving a destination conflict",
  );
  await expect(actionDialog(page, "Overwrite destination?")).toBeHidden();
  expect(renameRequests).toBe(1);
});

test("An expired rename source revision closes the editor and refreshes the listing", async ({
  appPage: page,
}) => {
  let listRequests = 0;
  let renameRequests = 0;
  page.on("request", request => {
    if (new URL(request.url()).pathname.endsWith("/__xczs__/api/list")) {
      listRequests++;
    }
  });
  await page.route("**/__xczs__/api/rename", route => {
    renameRequests++;
    return fulfillTrackedFailure(
      route,
      412,
      "source_changed",
      "The source changed after the list was loaded",
      { "X-Xczs-Source-Revision": "e".repeat(64) },
    );
  });

  await chooseFileAction(page, "rename", "rename-me.txt");
  const input = inlineNameInput(page);
  await input.fill("stale-source.txt");
  await input.press("Enter");
  const errorDialog = actionDialog(page, "Rename failed");
  await expect(errorDialog).toContainText(
    "The source changed after the list was loaded",
  );
  expect(listRequests).toBe(0);
  await errorDialog.getByRole("button", { name: "Close" }).click();

  await expect.poll(() => listRequests).toBeGreaterThan(0);
  await expect(inlineNameInput(page)).toHaveCount(0);
  await expect(rowByName(page, "rename-me.txt")).toBeVisible();
  expect(renameRequests).toBe(1);
});

test("An expired destination revision after overwrite confirmation refreshes without retry", async ({
  appPage: page,
}) => {
  let listRequests = 0;
  let renameRequests = 0;
  page.on("request", request => {
    if (new URL(request.url()).pathname.endsWith("/__xczs__/api/list")) {
      listRequests++;
    }
  });
  await page.route("**/__xczs__/api/rename", route => {
    renameRequests++;
    if (renameRequests === 1) {
      return fulfillTrackedFailure(
        route,
        409,
        "destination_exists",
        "The destination exists",
        { "X-Xczs-Target-Revision": "a".repeat(64) },
      );
    }
    return fulfillTrackedFailure(
      route,
      412,
      "destination_changed",
      "The destination changed after confirmation",
      { "X-Xczs-Target-Revision": "b".repeat(64) },
    );
  });

  await chooseFileAction(page, "rename", "overwrite-source.txt");
  const input = inlineNameInput(page);
  await input.fill("overwrite-target.txt");
  await input.press("Enter");
  await actionDialog(page, "Overwrite destination?")
    .getByRole("button", { name: "Overwrite" })
    .click();
  const errorDialog = actionDialog(page, "Rename failed");
  await expect(errorDialog).toContainText(
    "The destination changed after confirmation",
  );
  expect(listRequests).toBe(0);
  await errorDialog.getByRole("button", { name: "Close" }).click();

  await expect.poll(() => listRequests).toBeGreaterThan(0);
  await expect(inlineNameInput(page)).toHaveCount(0);
  await expect(rowByName(page, "overwrite-source.txt")).toBeVisible();
  expect(renameRequests).toBe(2);
});

test("An unknown rename result must not use the body conflict code to initiate overwrite", async ({
  appPage: page,
}) => {
  const source = currentLogicalChild(page, "overwrite-source.txt");
  let renameRequests = 0;
  let statusQueries = 0;
  await page.route("**/__xczs__/api/rename", route => {
    renameRequests++;
    const operationId = route.request().headers()["x-xczs-operation-id"];
    return route.fulfill({
      status: 409,
      contentType: "application/problem+json",
      headers: {
        "X-Xczs-Operation-Id": operationId,
        "X-Xczs-Operation-State": "unknown",
      },
      body: JSON.stringify({
        type: "urn:xczs:problem:destination_exists",
        title: "Conflict",
        status: 409,
        detail: "The target name may be occupied",
        code: "destination_exists",
        recovery: "query_job",
      }),
    });
  });
  await page.route("**/__xczs__/api/jobs/*", route => {
    statusQueries++;
    const operationId = new URL(route.request().url()).pathname.split("/").pop();
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      headers: {
        "X-Xczs-Operation-Id": operationId,
        "X-Xczs-Operation-State": "unknown",
      },
      body: JSON.stringify({
        job_id: operationId,
        state: "unknown",
        http_status: 409,
        code: "outcome_uncertain",
        detail: "The rename outcome is unknown",
      }),
    });
  });

  await chooseFileAction(page, "rename", "overwrite-source.txt");
  const input = inlineNameInput(page);
  await input.fill("overwrite-target.txt");
  await input.press("Enter");

  const errorDialog = actionDialog(page, "Rename failed");
  await expect(errorDialog).toContainText(
    "The server could not prove the final operation outcome",
  );
  expect({ renameRequests, statusQueries }).toEqual({
    renameRequests: 1,
    statusQueries: 1,
  });
  await expect(actionDialog(page, "Overwrite destination?")).toBeHidden();
});

test("Failed deletion displays an error and retains the directory row", async ({ appPage: page }) => {
  await page.route("**/delete-me.txt", async route => {
    if (route.request().method() === "DELETE") {
      await route.fulfill({
        status: 409,
        contentType: "application/problem+json",
        headers: {
          "X-Xczs-Operation-Id":
            route.request().headers()["x-xczs-operation-id"],
          "X-Xczs-Operation-State": "failed",
        },
        body: JSON.stringify({
          type: "urn:xczs:problem:forced_delete_failure",
          title: "Conflict",
          status: 409,
          code: "forced_delete_failure",
          detail: "forced delete failure",
        }),
      });
    } else {
      await route.continue();
    }
  });
  await chooseFileAction(page, "delete", "delete-me.txt");
  await actionDialog(page, "Delete item")
    .getByRole("button", { name: "Delete" })
    .click();
  const errorDialog = actionDialog(page, "Delete failed");
  await expect(errorDialog).toContainText('Unable to delete "delete-me.txt"');
  await expect(errorDialog).toContainText("forced delete failure");
  await expect(rowByName(page, "delete-me.txt")).toBeVisible();
  await expect(page.getByRole("button", {
    name: "Refresh",
    exact: true,
  })).toHaveCount(0);
  await errorDialog.getByRole("button", { name: "Close" }).click();
});

test("An expired deletion target revision refreshes without deleting", async ({
  appPage: page,
}) => {
  let deleteRequests = 0;
  let listRequests = 0;
  page.on("request", request => {
    if (new URL(request.url()).pathname.endsWith("/__xczs__/api/list")) {
      listRequests++;
    }
  });
  await page.route("**/delete-me.txt", route => {
    if (route.request().method() !== "DELETE") return route.continue();
    deleteRequests++;
    return fulfillTrackedFailure(
      route,
      412,
      "delete_target_changed",
      "The delete target changed after the list was loaded",
      { "X-Xczs-Source-Revision": "f".repeat(64) },
    );
  });

  await chooseFileAction(page, "delete", "delete-me.txt");
  await actionDialog(page, "Delete item")
    .getByRole("button", { name: "Delete" })
    .click();
  const errorDialog = actionDialog(page, "Delete failed");
  await expect(errorDialog).toContainText(
    "The delete target changed after the list was loaded",
  );
  expect(listRequests).toBe(0);
  await errorDialog.getByRole("button", { name: "Close" }).click();

  await expect.poll(() => listRequests).toBeGreaterThan(0);
  await expect(rowByName(page, "delete-me.txt")).toBeVisible();
  expect(deleteRequests).toBe(1);
});

test("A CSRF response after session rotation refreshes directly without querying an unregistered operation", async ({
  appPage: page,
}) => {
  let statusQueries = 0;
  await page.route("**/__xczs__/api/jobs/*", async route => {
    statusQueries++;
    await route.continue();
  });
  await rotateSession(page);

  await chooseFileAction(page, "delete", "delete-me.txt");
  const reloaded = page.waitForEvent("framenavigated", frame =>
    frame === page.mainFrame()
  );
  await actionDialog(page, "Delete item")
    .getByRole("button", { name: "Delete" })
    .click();
  await reloaded;

  await expect(page.locator(".index-page")).toBeVisible();
  await expect(rowByName(page, "delete-me.txt")).toBeVisible();
  expect(statusQueries).toBe(0);
});

test("After cancelled authentication reload, the next 401 requests reload again", async ({
  appPage: page,
}) => {
  let releaseUpload;
  let markUploadStarted;
  let mkdirRequests = 0;
  const uploadGate = new Promise(resolve => {
    releaseUpload = resolve;
  });
  const uploadStarted = new Promise(resolve => {
    markUploadStarted = resolve;
  });
  await page.route("**/reload-guard-upload.txt", async route => {
    if (route.request().method() !== "PUT") {
      await route.continue();
      return;
    }
    markUploadStarted();
    await uploadGate;
    await route.fulfill({
      status: 201,
      headers: {
        "X-Xczs-Upload-Id":
          route.request().headers()["x-xczs-upload-id"],
        "X-Xczs-Upload-Length":
          route.request().headers()["x-xczs-upload-length"],
        "X-Xczs-Upload-Offset":
          route.request().headers()["x-xczs-upload-length"],
        "X-Xczs-Operation-State": "committed",
      },
      body: "",
    });
  });
  await page.route("**/__xczs__/api/mkdir", route => {
    mkdirRequests++;
    return route.fulfill({
      status: 401,
      contentType: "application/json",
      body: JSON.stringify({ code: "auth.session_required", message: "Sign in again", retryable: false }),
    });
  });

  const browserDialogTypes = [];
  page.on("dialog", async dialog => {
    browserDialogTypes.push(dialog.type());
    await dialog.dismiss();
  });
  await selectFiles(page, "#file", [{
    name: "reload-guard-upload.txt",
    buffer: Buffer.from("keep the beforeunload guard active"),
  }]);
  await uploadStarted;

  const newFolder = page.getByRole("button", { name: "New folder" });
  await newFolder.click();
  await expect.poll(() => browserDialogTypes).toEqual(["beforeunload"]);
  await expect(newFolder).not.toHaveAttribute("aria-busy", "true");

  await newFolder.click();
  await expect.poll(() => browserDialogTypes).toEqual([
    "beforeunload",
    "beforeunload",
  ]);
  expect(mkdirRequests).toBe(2);

  releaseUpload();
  await expect(page.locator(".upload-status")).toHaveAttribute(
    "aria-label",
    "reload-guard-upload.txt: upload complete",
  );
});

test("An uncertain submission outcome is queried once without blind retries", async ({ appPage: page }) => {
  await page.clock.install();
  let releaseRequest;
  let operationId = "";
  let deleteRequests = 0;
  let statusQueries = 0;
  const requestGate = new Promise(resolve => {
    releaseRequest = resolve;
  });
  await page.route("**/delete-me.txt", async route => {
    if (route.request().method() !== "DELETE") {
      await route.continue();
      return;
    }
    deleteRequests++;
    operationId = route.request().headers()["x-xczs-operation-id"];
    await requestGate;
    try {
      await route.fulfill({ status: 204, body: "" });
    } catch {
      // The client-side deadline is expected to close the intercepted request.
    }
  });
  await page.route("**/__xczs__/api/jobs/*", async route => {
    statusQueries++;
    expect(new URL(route.request().url()).pathname).toContain(operationId);
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      headers: {
        "X-Xczs-Operation-Id": operationId,
        "X-Xczs-Operation-State": "unknown",
      },
      body: JSON.stringify({
        job_id: operationId,
        state: "unknown",
        http_status: 500,
        code: "outcome_uncertain",
        detail: "Operation outcome is uncertain",
      }),
    });
  });
  await chooseFileAction(page, "delete", "delete-me.txt");
  await actionDialog(page, "Delete item")
    .getByRole("button", { name: "Delete" })
    .click();
  await expect.poll(() => deleteRequests).toBe(1);
  await page.clock.fastForward(30 * 1000 + 1);
  const errorDialog = actionDialog(page, "Delete failed");
  await expect(errorDialog).toContainText("could not prove");
  await expect(page.getByRole("button", {
    name: "Refresh",
    exact: true,
  })).toBeVisible();
  await expect(page.locator(".list-status")).toContainText(
    "Folder contents may have changed",
  );
  expect(deleteRequests).toBe(1);
  expect(statusQueries).toBe(1);
  await expect(rowByName(page, "delete-me.txt")).toBeVisible();
  await errorDialog.getByRole("button", { name: "Close" }).click();
  releaseRequest();
});

test("A successful status query updates the page safely", async ({ appPage: page }) => {
  await page.clock.install();
  let releaseRequest;
  let operationId = "";
  let backendDeleteStatus = 0;
  const jobResponses = [];
  const requestGate = new Promise(resolve => {
    releaseRequest = resolve;
  });
  await page.route("**/delete-me.txt", async route => {
    if (route.request().method() !== "DELETE") {
      await route.continue();
      return;
    }
    operationId = route.request().headers()["x-xczs-operation-id"];
    const response = await fetchSameOriginRoute(route);
    backendDeleteStatus = response.status();
    await requestGate;
    try {
      await route.fulfill({ response });
    } catch {
      // The client-side deadline is expected to close the intercepted request.
    }
  });
  page.on("response", response => {
    if (new URL(response.url()).pathname.startsWith("/__xczs__/api/jobs/")) {
      jobResponses.push(response);
    }
  });
  await chooseFileAction(page, "delete", "delete-me.txt");
  await actionDialog(page, "Delete item")
    .getByRole("button", { name: "Delete" })
    .click();
  await expect.poll(() => backendDeleteStatus).toBe(204);
  await page.clock.fastForward(30 * 1000 + 1);
  await expect(rowByName(page, "delete-me.txt")).toHaveCount(0);
  expect(jobResponses).toHaveLength(1);
  expect(new URL(jobResponses[0].url()).pathname).toContain(operationId);
  expect(jobResponses[0].status()).toBe(200);
  releaseRequest();
});

test("Job queries reject contradictory headers and invalid success status codes", async ({ appPage: page }) => {
  let responseVariant = 0;
  await page.route("**/__xczs__/api/jobs/*", route => {
    const operationId = new URL(route.request().url()).pathname.split("/").pop();
    responseVariant++;
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      headers: {
        "X-Xczs-Operation-Id": operationId,
        "X-Xczs-Operation-State":
          responseVariant === 1 ? "unknown" : "succeeded",
      },
      body: JSON.stringify({
        job_id: operationId,
        state: "succeeded",
        http_status: responseVariant === 1 ? 204 : 500,
      }),
    });
  });

  const results = await page.evaluate(async () => {
    const entrypoint =
      document.querySelector('script[type="module"]').src;
    const { queryJob } = await import(
      new URL("modules/http/client.js", entrypoint).href
    );
    return [
      await queryJob("00000000-0000-4000-8000-000000000010"),
      await queryJob("00000000-0000-4000-8000-000000000011"),
    ];
  });
  for (const result of results) {
    expect(result).toMatchObject({
      kind: "unavailable",
      state: "unknown",
      code: "job_status_unavailable",
      authenticationFailed: false,
    });
  }
});

test("Successful deletion updates the loaded count and moves focus", async ({ appPage: page }) => {
  let listingRequests = 0;
  await page.route("**/__xczs__/api/list?**", route => {
    listingRequests++;
    return listingRequests === 1
      ? fulfillDirectoryChanged(route)
      : route.continue();
  });
  const status = page.locator(".list-status");
  const beforeText = await status.textContent();
  const beforeCount = Number(beforeText.match(/\d+/)?.[0]);
  expect(beforeCount).toBeGreaterThan(0);

  const responsePromise = page.waitForResponse(
    response =>
      response.request().method() === "DELETE" &&
      new URL(response.url()).pathname.endsWith("/delete-me.txt"),
  );
  await chooseFileAction(page, "delete", "delete-me.txt");
  await actionDialog(page, "Delete item")
    .getByRole("button", { name: "Delete" })
    .click();
  const response = await responsePromise;
  expect(response.status()).toBe(204);
  expect(response.request().headers()["if-match"]).toMatch(
    /^"[0-9a-f]{64}"$/,
  );
  await expect(rowByName(page, "delete-me.txt")).toHaveCount(0);
  await expect(status).toHaveText(`All ${beforeCount - 1} items loaded`);
  expect(listingRequests).toBe(2);
  expect(
    await page.evaluate(() => document.activeElement !== document.body),
  ).toBe(true);
});

test("Consecutive directory-change conflicts retry automatically only once", async ({ appPage: page }) => {
  let listingRequests = 0;
  await page.route("**/__xczs__/api/list?**", route => {
    listingRequests++;
    return fulfillDirectoryChanged(route);
  });

  await page.reload();
  await expect(page.locator(".list-status")).toHaveText(
    "Unable to load the file list: Directory changed; restart listing",
  );
  await expect(page.getByRole("button", { name: "Retry" })).toBeEnabled();
  expect(listingRequests).toBe(2);
});

test("Creating a pinned entry during pending writes must not let an old index delete the wrong row", async ({
  appPage: page,
}) => {
  let releaseDelete;
  let deleteTarget = "";
  let renameRequests = 0;
  const deleteGate = new Promise(resolve => {
    releaseDelete = resolve;
  });
  await page.route("**/delete-me.txt", async route => {
    if (route.request().method() !== "DELETE") {
      await route.continue();
      return;
    }
    deleteTarget = decodeURIComponent(new URL(route.request().url()).pathname);
    await deleteGate;
    await route.continue();
  });
  page.on("request", request => {
    if (
      request.method() === "POST" &&
      new URL(request.url()).pathname.endsWith("/__xczs__/api/rename")
    ) {
      renameRequests++;
    }
  });

  const originalIndex = await rowByName(page, "delete-me.txt").getAttribute("data-index");
  await chooseFileAction(page, "delete", "delete-me.txt");
  await actionDialog(page, "Delete item")
    .getByRole("button", { name: "Delete" })
    .click();
  await expect(page.locator(".operation-status")).toContainText(
    "Deleting delete-me.txt",
  );

  await page.getByRole("button", { name: "New folder" }).click();
  const newItemEditor = inlineNameInput(page);
  await expect(newItemEditor).toHaveValue("newfolder");
  await newItemEditor.fill("unsaved-newfolder");
  await expect(rowByName(page, "delete-me.txt")).not.toHaveAttribute("data-index", originalIndex);
  await expect(newItemEditor).toBeFocused();
  expect(renameRequests).toBe(0);

  releaseDelete();
  await expect(rowByName(page, "delete-me.txt")).toHaveCount(0);
  await expect(newItemEditor).toHaveValue("unsaved-newfolder");
  await expect(newItemEditor).toBeFocused();
  expect(renameRequests).toBe(0);

  await newItemEditor.press("Escape");
  await expect(newItemEditor).toHaveCount(0);
  await expect(rowByName(page, "newfolder")).toBeVisible();
  await expect(rowByName(page, "unsaved-newfolder")).toHaveCount(0);
  await expect(rowByName(page, "download-me.txt")).toBeVisible();
  expect(deleteTarget).toBe(`${currentDirectoryPath(page)}/delete-me.txt`);
  expect(renameRequests).toBe(0);
});

test("Invalid inline names show an inline error without rename requests", async ({ appPage: page }) => {
  let renameRequests = 0;
  page.on("request", request => {
    if (
      request.method() === "POST" &&
      new URL(request.url()).pathname.endsWith("/__xczs__/api/rename")
    ) {
      renameRequests++;
    }
  });
  await chooseFileAction(page, "rename", "rename-me.txt");
  const input = inlineNameInput(page);
  await input.fill("..");
  await input.press("Enter");
  await expect(input).toHaveAttribute("aria-invalid", "true");
  await expect(page.locator(".inline-name-error")).toHaveAttribute(
    "role",
    "alert",
  );
  await expect(page.locator(".inline-name-error")).not.toBeEmpty();
  expect(await input.evaluate(element => {
    const descriptionId = element.getAttribute("aria-describedby");
    return Boolean(
      descriptionId &&
      document.getElementById(descriptionId)?.classList.contains("inline-name-error"),
    );
  })).toBe(true);
  expect(renameRequests).toBe(0);
  await expect(page.getByRole("button", {
    name: "Refresh",
    exact: true,
  })).toHaveCount(0);
  await page.getByRole("button", { name: "New folder" }).focus();
  await expect(inlineNameInput(page)).toHaveCount(0);
  await expect(rowByName(page, "rename-me.txt")).toBeVisible();
  expect(renameRequests).toBe(0);
});

test("Long-running operations expose busy state and retain focus capability for the triggering control", async ({
  appPage: page,
}) => {
  let releaseDelete;
  const deleteGate = new Promise(resolve => {
    releaseDelete = resolve;
  });
  await page.route("**/download-me.txt", async route => {
    if (route.request().method() !== "DELETE") {
      await route.continue();
      return;
    }
    await deleteGate;
    await route.fulfill({
      status: 204,
      headers: {
        "X-Xczs-Operation-Id":
          route.request().headers()["x-xczs-operation-id"],
        "X-Xczs-Operation-State": "succeeded",
      },
      body: "",
    });
  });
  const remove = page.locator('[data-file-action="delete"]');
  await chooseFileAction(page, "delete", "download-me.txt");
  await actionDialog(page, "Delete item")
    .getByRole("button", { name: "Delete" })
    .click();
  await expect(remove).toHaveAttribute("aria-busy", "true");
  await expect(remove).toHaveAttribute("aria-disabled", "true");
  await expect(page.locator(".operation-status")).toHaveText(
    "Deleting download-me.txt…",
  );
  releaseDelete();
  await expect(rowByName(page, "download-me.txt")).toHaveCount(0);
  await expect(page.locator(".operation-status")).toBeHidden();
});
