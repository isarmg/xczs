const { currentLogicalChild, expect, selectFiles, test } = require("./fixtures");

function protocolHeaders(request, state, offset = null, storedLength = null) {
  const headers = request.headers();
  const length = storedLength ?? headers["x-xczs-upload-length"];
  return {
    "X-Xczs-Upload-Id": headers["x-xczs-upload-id"],
    "X-Xczs-Operation-State": state,
    ...(length === undefined ? {} : { "X-Xczs-Upload-Length": String(length) }),
    ...(offset === null ? {} : { "X-Xczs-Upload-Offset": offset === "length" ? String(length) : String(offset) }),
  };
}

function problem(status, code) {
  return JSON.stringify({ type: `urn:xczs:problem:${code}`, title: "Upload failed", status, code, detail: "Fixture cleanup response" });
}

async function stagedSkip(page, { name, checkpoint, csrf = false }) {
  await expect(page.locator("html")).toHaveAttribute("data-xcss-runtime", "leptos");
  const contents = Buffer.from("staged upload cleanup contract");
  const methods = [], discards = [], preflights = [];
  let uploadId;
  page.on("request", request => {
    if (new URL(request.url()).pathname === "/__xczs__/api/upload/preflight") preflights.push(request.postDataJSON());
  });
  // Preflight stays on the real Axum backend. Only the bound upload outcome and
  // cleanup reply are injected, matching the existing staged-conflict tests.
  await page.route(`**/${name}`, async route => {
    const request = route.request();
    methods.push(request.method());
    if (request.method() === "HEAD") {
      expect(csrf).toBe(false);
      expect(request.headers()["x-xczs-upload-id"]).toBe(uploadId);
      return route.fulfill({
        status: checkpoint === "not-seen" ? 404 : checkpoint === "rejected" ? 409 : 200,
        headers: checkpoint === "not-seen"
          ? protocolHeaders(request, "not-seen")
          : protocolHeaders(request, checkpoint, "length", contents.length),
        body: "",
      });
    }
    expect(request.method()).toBe("PUT");
    expect(methods).toEqual(["PUT"]);
    expect(request.headers()["x-xczs-upload-overwrite"]).toBe("false");
    expect(request.postDataBuffer()).toEqual(contents);
    uploadId = request.headers()["x-xczs-upload-id"];
    return route.fulfill({
      status: 409,
      contentType: "application/problem+json",
      headers: {
        ...protocolHeaders(request, "awaiting-confirmation", "length"),
        "X-Xczs-Target-Revision": "f".repeat(64),
        "X-Xczs-Target-Replaceable": "true",
      },
      body: problem(409, "destination_exists"),
    });
  });
  await page.route("**/__xczs__/api/upload/discard", async route => {
    const request = route.request();
    expect(request.method()).toBe("POST");
    const payload = request.postDataJSON();
    discards.push(payload);
    expect(payload).toEqual({ path: currentLogicalChild(page, name), upload_id: uploadId });
    // An unbound 204 cannot prove discard; one read-only checkpoint is required.
    return route.fulfill(csrf
      ? { status: 403, contentType: "application/json", body: JSON.stringify({ code: "auth.csrf_rejected", message: "Request rejected", retryable: false }) }
      : { status: 204, body: "" });
  });
  await selectFiles(page, "#file", [{ name, buffer: contents }]);
  const changed = page.getByRole("dialog", { name: "Upload destination changed", exact: true });
  await expect(changed).toContainText("uploaded data is staged");
  await changed.getByRole("button", { name: "Skip file", exact: true }).click();
  return { methods, discards, preflights };
}

for (const checkpoint of ["rejected", "not-seen"]) {
  test(`Leptos untrusted staged discard reconciles ${checkpoint} with one HEAD before reporting skipped`, async ({ appPage: page }) => {
    const name = `cleanup-${checkpoint}.txt`;
    const { methods, discards, preflights } = await stagedSkip(page, { name, checkpoint });
    await expect(page.locator(".upload-status")).toHaveAttribute("aria-label", `${name}: skipped because the destination exists`);
    expect(methods).toEqual(["PUT", "HEAD"]);
    expect(discards).toHaveLength(1);
    expect(preflights).toHaveLength(1);
    await expect(page.locator(".upload-status .upload-cancel, .upload-status .retry-btn")).toHaveCount(0);
  });
}

test("Leptos staged cleanup checkpoint committed reports completion rather than skipped", async ({ appPage: page }) => {
  const name = "cleanup-committed.txt";
  const { methods, discards, preflights } = await stagedSkip(page, { name, checkpoint: "committed" });
  await expect(page.locator(".upload-status")).toHaveAttribute("aria-label", `${name}: upload complete`);
  expect(methods).toEqual(["PUT", "HEAD"]);
  expect(discards).toHaveLength(1);
  expect(preflights).toHaveLength(1);
  await expect(page.locator(".upload-status")).not.toContainText("Skipped");
});

test("Leptos staged discard CSRF failure pauses admission without reload or HEAD", async ({ appPage: page }) => {
  const name = "cleanup-csrf.txt";
  const identity = await page.evaluate(() => window.cleanupDocumentIdentity = crypto.randomUUID());
  const { methods, discards, preflights } = await stagedSkip(page, { name, csrf: true });
  await expect(page.locator(".upload-queue-message")).toContainText("Your session or this page is no longer valid");
  await expect(page.locator(".upload-status .upload-failure")).toBeVisible();
  await selectFiles(page, "#file", [{ name: "must-remain-paused.txt", buffer: Buffer.from("must not send") }]);
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  expect(await page.evaluate(() => window.cleanupDocumentIdentity)).toBe(identity);
  expect(methods).toEqual(["PUT"]);
  expect(discards).toHaveLength(1);
  expect(preflights).toHaveLength(1);
  await expect(page.locator(".uploader")).toHaveCount(1);
  await expect(page.locator(".upload-status .retry-btn")).toHaveCount(0);
});
