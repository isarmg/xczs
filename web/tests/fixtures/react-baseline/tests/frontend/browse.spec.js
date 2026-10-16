const {
  actionDialog,
  chooseFileAction,
  currentLogicalChild,
  currentUrl,
  expect,
  login,
  sameOriginRequestHeaders,
  test,
} = require("./fixtures");

const dangerousName = `危险 <img src=x onerror=alert(1)> & "'.txt`;
const listingRevision = "a".repeat(64);

test("Dangerous filenames always render as plain text nodes", async ({ appPage: page }) => {
  const link = page.getByRole("link", { name: dangerousName, exact: true });
  await expect(link).toBeVisible();
  expect(await link.textContent()).toBe(dangerousName);
  await expect(page.locator(".paths-table img")).toHaveCount(0);
  expect(
    await page.locator(".paths-table tbody").evaluate(element =>
      element.querySelector("[onerror], script, iframe") !== null
    ),
  ).toBe(false);
});

test("Search preserves the entire query containing special characters", async ({ appPage: page }) => {
  const query = "special & # + 中文";
  await page.getByLabel("Search files or folders").fill(query);
  await Promise.all([
    page.waitForURL(url => url.searchParams.get("q") === query),
    page.getByLabel("Search files or folders").press("Enter"),
  ]);
  await expect(
    page.getByRole("link", { name: "special & # + 中文.txt", exact: true }),
  ).toBeVisible();
});

test("Sort links propagate only supported parameters and expose the sort state", async ({ appPage: page }) => {
  const target = new URL(page.url());
  target.search = "?q=existing&unused=1";
  await page.goto(target.href);
  const nameHeader = page.locator(".paths-table thead th").first();
  const link = nameHeader.getByRole("link");
  const url = new URL(await link.getAttribute("href"), page.url());
  expect([...url.searchParams.keys()].sort()).toEqual(["order", "q", "sort"]);
  expect(url.searchParams.get("q")).toBe("existing");
  expect(url.searchParams.get("sort")).toBe("name");
  expect(url.searchParams.get("order")).toBe("desc");
  await expect(nameHeader).toHaveAttribute("aria-sort", "ascending");
});

test("The directory API loads at most 200 entries per page and supports appending more pages", async ({ page }, testInfo) => {
  let calls = 0;
  await page.route("**/__xczs__/api/list?**", async route => {
    calls++;
    const url = new URL(route.request().url());
    expect(url.searchParams.get("path")).toBe("/");
    expect(url.searchParams.get("limit")).toBe("200");
    const cursor = url.searchParams.get("cursor");
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(cursor === null
        ? {
          paths: [
            { path_type: "Dir", name: "existing-folder", mtime: 0, size: 0, revision: listingRevision },
            { path_type: "File", name: "page-one.txt", mtime: 0, size: 1, revision: listingRevision },
          ],
          next_cursor: "opaque-next",
        }
        : {
          paths: [
            { path_type: "File", name: "page-two.txt", mtime: 0, size: 2, revision: listingRevision },
          ],
          next_cursor: null,
        }),
    });
  });

  await login(page, testInfo.parallelIndex);
  await expect(page.getByRole("link", {
    name: "Download folder existing-folder",
  })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Load more" })).toBeVisible();
  await page.getByRole("button", { name: "Load more" }).click();
  await expect(
    page.getByRole("link", { name: "page-two.txt", exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Load more" })).toBeHidden();
  await expect(page.locator(".list-status")).toHaveText("All 3 items loaded");
  await expect(page.locator(".list-status")).toBeFocused();
  expect(calls).toBe(2);
});

test("Directory requests abort on timeout and expose a retryable state", async ({ appPage: page }) => {
  let calls = 0;
  let releaseRequest;
  const requestGate = new Promise(resolve => {
    releaseRequest = resolve;
  });
  await page.route("**/__xczs__/api/list?**", async route => {
    calls++;
    if (calls === 1) {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          paths: [{
            path_type: "File",
            name: "first-page.txt",
            mtime: 0,
            size: 1,
            revision: listingRevision,
          }],
          next_cursor: "timeout-page",
        }),
      });
      return;
    }
    await requestGate;
    try {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ paths: [], next_cursor: null }),
      });
    } catch {
      // The client-side deadline is expected to close the intercepted request.
    }
  });

  await page.reload();
  const loadMore = page.getByRole("button", { name: "Load more" });
  await expect(loadMore).toBeVisible();
  await page.clock.install();
  await loadMore.click();
  await page.clock.fastForward(30 * 1000 + 1);
  await expect(page.locator(".list-status")).toContainText(
    "Unable to load the file list: Request timed out. Try again.",
  );
  await expect(page.getByRole("button", { name: "Retry" })).toBeEnabled();
  releaseRequest();
});

test("Missing directories display an uploadable empty state without requesting a listing", async ({
  appPage: page,
}) => {
  let listRequests = 0;
  page.on("request", request => {
    if (new URL(request.url()).pathname.endsWith("/__xczs__/api/list")) {
      listRequests++;
    }
  });
  await page.goto(currentUrl(page, "not-created-yet/"));
  await expect(page.locator(".empty-folder")).toHaveText(
    "Uploading files will create this folder automatically",
  );
  await expect(page.locator(".list-status")).toBeEmpty();
  expect(listRequests).toBe(0);
});

test("Search accepts 128 multibyte characters", async ({ appPage: page }) => {
  const query = "文".repeat(128);
  const responsePromise = page.waitForResponse(response => {
    const url = new URL(response.url());
    return url.pathname.endsWith("/__xczs__/api/list") &&
      url.searchParams.get("q") === query;
  });
  await page.getByLabel("Search files or folders").fill(query);
  await page.getByLabel("Search files or folders").press("Enter");
  expect((await responsePromise).status()).toBe(200);
  await expect(page.locator(".empty-folder")).toHaveText("No search results");
  await expect(page.locator(".list-status")).not.toContainText("Unable to load");
});

test("File links download attachments and preserve Range downloads", async ({ appPage: page }) => {
  const target = currentUrl(page, "download-me.txt");
  const fileLink = page.getByRole("link", {
    name: "download-me.txt",
    exact: true,
  });
  const downloadPromise = page.waitForEvent("download");
  await fileLink.click();
  const download = await downloadPromise;
  expect(download.suggestedFilename()).toBe("download-me.txt");
  const stream = await download.createReadStream();
  const chunks = [];
  for await (const chunk of stream) chunks.push(chunk);
  expect(Buffer.concat(chunks).toString()).toBe("downloaded by browser test");

  expect(
    await page.evaluate(async url => {
      const response = await fetch(url, {
        headers: { Range: "bytes=0-4" },
      });
      return {
        status: response.status,
        range: response.headers.get("content-range"),
        body: await response.text(),
      };
    }, target),
  ).toEqual({
    status: 206,
    range: "bytes 0-4/26",
    body: "downl",
  });
});

test("A directory page does not commit partial DOM when whole-page validation fails", async ({ appPage: page }) => {
  await page.route("**/__xczs__/api/list?**", route => route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify({
      paths: [
        { path_type: "File", name: "valid-before-error.txt", mtime: 0, size: 1, revision: listingRevision },
        { path_type: "File", name: "../invalid.txt", mtime: 0, size: 1, revision: listingRevision },
      ],
      next_cursor: null,
    }),
  }));
  await page.reload();
  await expect(page.locator(".list-status")).toContainText(
    "Unable to load the file list: Invalid file list item",
  );
  await expect(page.getByRole("link", {
    name: "valid-before-error.txt",
  })).toHaveCount(0);
  await expect(page.locator(".paths-table tbody tr")).toHaveCount(0);
});

test("Directory pagination rejects duplicate cursors and preserves the previous page", async ({ appPage: page }) => {
  let calls = 0;
  await page.route("**/__xczs__/api/list?**", route => {
    calls++;
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        paths: [{
          path_type: "File",
          name: calls === 1 ? "cursor-page-one.txt" : "cursor-page-two.txt",
          mtime: 0,
          size: 1,
          revision: listingRevision,
        }],
        next_cursor: "repeated-cursor",
      }),
    });
  });
  await page.reload();
  await page.getByRole("button", { name: "Load more" }).click();
  await expect(page.locator(".list-status")).toContainText(
    "The server repeated a file list cursor",
  );
  await expect(page.getByRole("link", {
    name: "cursor-page-one.txt",
    exact: true,
  })).toBeVisible();
  await expect(page.getByRole("link", {
    name: "cursor-page-two.txt",
    exact: true,
  })).toHaveCount(0);
});

test("Large directories limit DOM entries with an accessible window", async ({ appPage: page }) => {
  test.slow();
  let created = false;
  await page.route("**/__xczs__/api/list?**", route => {
    const cursor = new URL(route.request().url()).searchParams.get("cursor");
    const current = cursor === null ? 0 : Number(cursor.slice("cursor-".length));
    const paths = Array.from({ length: 200 }, (_, offset) => ({
      path_type: "File",
      name: `window-${current * 200 + offset}.txt`,
      mtime: 0,
      size: offset,
      revision: listingRevision,
    }));
    // Closing the created-item editor refreshes the first page. Keep that
    // inserted file ahead of window-0 so every toolbar action must resolve the
    // shifted row by its current identity, including across this refresh.
    if (created && current === 0) {
      paths.unshift({
        path_type: "File", name: "newfile", mtime: 0, size: 0,
        revision: listingRevision,
      });
      paths.pop();
    }
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        paths,
        next_cursor: current < 2 ? `cursor-${current + 1}` : null,
      }),
    });
  });
  await page.reload();
  for (let index = 0; index < 1; index++) {
    await page.getByRole("button", { name: "Load more" }).click();
    await expect(page.locator(".list-status")).toContainText(
      `${(index + 2) * 200} items loaded`,
    );
  }
  await expect(page.locator(".paths-table tbody tr")).toHaveCount(200);
  const previous = page.getByRole("button", { name: "Show previous items" });
  const listStatus = page.locator(".list-status");
  await expect(previous).toBeVisible();
  await previous.click();
  await expect(listStatus).toBeFocused();
  await expect(page.getByRole("link", {
    name: "window-0.txt",
    exact: true,
  })).toBeVisible();
  await expect(page.getByRole("button", { name: "Show next items" })).toBeVisible();

  await page.getByRole("button", { name: "New empty file" }).click();
  created = true;
  const inlineEditor = page.locator(".inline-name-input");
  await expect(inlineEditor).toHaveValue("newfile");
  await expect(inlineEditor).toBeFocused();
  await expect(page.locator(".paths-table tbody tr")).toHaveCount(200);
  await expect(page.locator(".paths-table tbody tr.is-renaming")).toHaveCount(1);
  const shiftedFirstRow = page.locator("#addPath1");
  await expect(shiftedFirstRow.getByRole("link", {
    name: "window-0.txt",
    exact: true,
  })).toBeVisible();
  await expect(shiftedFirstRow).toHaveAttribute("data-index", "1");
  await expect(page.getByRole("link", {
    name: "window-199.txt",
    exact: true,
  })).toHaveCount(0);

  const source = currentLogicalChild(page, "window-0.txt");
  const destination = currentLogicalChild(page, "existing-folder");
  const requests = [];
  page.on("request", request => {
    const path = new URL(request.url()).pathname;
    if (request.method() === "DELETE" ||
        (request.method() === "POST" && /\/__xczs__\/api\/(move|rename)$/.test(path))) {
      requests.push(request);
    }
  });

  for (const action of ["move", "delete", "rename"]) {
    await chooseFileAction(page, action, "window-0.txt");
    const mode = page.locator(`[data-file-action="${action}"]`);
    await expect(mode).toHaveAttribute("aria-pressed", "false");
    const mutationUrl = action === "delete"
      ? currentUrl(page, "window-0.txt")
      : new URL(`/__xczs__/api/${action}`, page.url()).href;
    const responseBodies = new WeakMap();
    // Retain the real server body before forwarding the same response to the UI.
    // Chromium's DevTools cache can discard these private, no-store bodies even
    // without navigation, so starting response.json() immediately is insufficient.
    const captureResponse = async route => {
      const request = route.request();
      if (request.method() !== (action === "delete" ? "DELETE" : "POST")) {
        return route.fallback();
      }
      const headers = await request.allHeaders();
      const proof = sameOriginRequestHeaders(page);
      headers.origin = proof.Origin;
      headers["sec-fetch-site"] = proof["Sec-Fetch-Site"];
      const upstream = await route.fetch({ headers, maxRedirects: 0, maxRetries: 0 });
      try {
        const body = await upstream.body();
        responseBodies.set(request, body);
        await route.fulfill({ response: upstream, body });
      } finally {
        await upstream.dispose();
      }
    };
    await page.route(mutationUrl, captureResponse, { times: 1 });
    const [{ response, payload }] = await Promise.all([
      page.waitForResponse(response => {
        const request = response.request();
        return action === "delete"
          ? request.method() === "DELETE"
          : request.method() === "POST" &&
            new URL(response.url()).pathname === `/__xczs__/api/${action}`;
      }).then(response => ({
        response,
        payload: JSON.parse(responseBodies.get(response.request()).toString("utf8")),
      })),
      (async () => {
        if (action === "rename") {
          await expect(inlineEditor).toHaveValue("window-0.txt");
          await expect(inlineEditor).toBeFocused();
          await expect(page.locator("#addPath1.is-renaming")).toHaveCount(1);
          await inlineEditor.fill("window-renamed.txt");
          await inlineEditor.press("Enter");
        } else {
          const dialog = actionDialog(page, action === "move" ? "Move item" : "Delete item");
          await expect(dialog).toBeVisible();
          if (action === "move") {
            const input = dialog.getByRole("textbox", { name: "Destination folder" });
            await expect(input).toBeFocused();
            await input.fill(destination);
          } else {
            await expect(dialog).toContainText('Delete "window-0.txt"?');
          }
          await dialog.getByRole("button", {
            name: action === "move" ? "Move" : "Delete", exact: true,
          }).click();
        }
      })(),
    ]).finally(() => page.unroute(mutationUrl, captureResponse));
    // The window rows are synthetic, so the actual fixture backend must reject
    // these operations. A stale index must never mutate the real newfile.
    expect(response.status()).toBe(412);
    expect(payload.code).toBe(
      action === "delete" ? "delete_target_changed" : "source_changed",
    );
    expect(response.headers()["x-xczs-operation-state"]).toBe("failed");
    const request = response.request();
    expect(request.headers()["x-csrf-token"]).toBeTruthy();
    expect(request.headers()["x-xczs-operation-id"]).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
    );
    if (action === "delete") {
      expect(decodeURIComponent(new URL(request.url()).pathname)).toBe(source);
      expect(request.headers()["if-match"]).toBe(`"${listingRevision}"`);
    } else {
      expect(request.postDataJSON()).toEqual(action === "move"
        ? { source, directory: destination, source_revision: listingRevision, overwrite: false }
        : { source, name: "window-renamed.txt", source_revision: listingRevision, overwrite: false });
    }
    const failure = actionDialog(page, action === "move" ? "Move failed" :
      action === "delete" ? "Delete failed" : "Rename failed");
    await expect(failure).toContainText("window-0.txt");
    await failure.getByRole("button", { name: "Close", exact: true }).click();
    if (action === "rename") {
      await expect(inlineEditor).toHaveCount(0);
    }
    await expect(mode).toBeFocused();
    await expect(shiftedFirstRow).toHaveAttribute("data-index", "1");
    await expect(shiftedFirstRow.getByRole("link", {
      name: "window-0.txt", exact: true,
    })).toBeVisible();
    await expect(page.locator(".paths-table tbody tr")).toHaveCount(200);
    const newfile = await page.context().request.get(currentUrl(page, "newfile"));
    expect(newfile.status()).toBe(200);
    expect(await newfile.body()).toHaveLength(0);
  }
  expect(requests.map(request => request.method())).toEqual(["POST", "DELETE", "POST"]);
});

test("Sorting retains filters changed within the current document and after Back", async ({ appPage: page }) => {
  const search = page.getByLabel("Search files or folders");
  await search.fill("existing");
  await search.press("Enter");
  await expect.poll(() => new URL(page.url()).searchParams.get("q")).toBe("existing");
  let size = page.locator(".paths-table thead .cell-size a");
  await expect.poll(async () => new URL(await size.getAttribute("href"), page.url()).searchParams.get("q")).toBe("existing");
  await search.fill("download");
  await search.press("Enter");
  await expect.poll(() => new URL(page.url()).searchParams.get("q")).toBe("download");
  await page.goBack();
  await expect(search).toHaveValue("existing");
  size = page.locator(".paths-table thead .cell-size a");
  await expect.poll(async () => new URL(await size.getAttribute("href"), page.url()).searchParams.get("q")).toBe("existing");
  await size.click();
  await expect.poll(() => new URL(page.url()).searchParams.get("sort")).toBe("size");
  expect(new URL(page.url()).searchParams.get("q")).toBe("existing");
});
