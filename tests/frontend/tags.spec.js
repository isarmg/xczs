const { test, expect, sameOriginRequestHeaders } = require("./fixtures.js");
const { randomUUID } = require("node:crypto");

test.beforeEach(async ({ appPage: page }) => {
  await page.evaluate(() =>
    localStorage.setItem("sarmg.admin.language", "zh-CN"),
  );
  await page.reload();
  await expect(page.locator(".paths-table tbody tr").first()).toBeVisible();
});

async function observeJsonResponses(page) {
  const payloads = new WeakMap();
  // Forward each request once to the real server and retain its exact body.
  // Chromium may discard no-store bodies from its DevTools response cache.
  await page.route("**/api/v1/file-tags/**", async (route) => {
    const headers = await route.request().allHeaders();
    const proof = sameOriginRequestHeaders(page);
    headers.origin = proof.Origin;
    headers["sec-fetch-site"] = proof["Sec-Fetch-Site"];
    const upstream = await route.fetch({
      maxRedirects: 0,
      headers,
    });
    const body = await upstream.body();
    payloads.set(
      route.request(),
      body.length ? JSON.parse(body.toString("utf8")) : undefined,
    );
    await route.fulfill({ response: upstream, body });
    await upstream.dispose();
  });
  return async (predicate) => {
    const response = await page.waitForResponse(predicate);
    return { response, payload: payloads.get(response.request()) };
  };
}

async function expectSharedNavigation(page) {
  const links = page.locator(".xcss-header-navigation a");
  await expect(links).toHaveCount(3);
  expect(
    await links.evaluateAll((nodes) =>
      nodes.map((node) => node.getAttribute("href")),
    ),
  ).toEqual(["#files", "#tags", "#status"]);
}

async function ensureIndexed(bar) {
  const scan = bar.getByRole("button", { name: "扫描整个文件库", exact: true });
  await expect.poll(async () => await scan.count() + await bar.locator(".file-tag-options").count()).toBeGreaterThan(0);
  if (await scan.count()) {
    await expect(bar).toContainText("整个文件库");
    await scan.click();
  }
  await expect(bar.locator(".file-tag-options")).toBeVisible();
}

test("The directory table displays a tag column and tag search conditions fit narrow screens", async ({
  appPage: page,
}, testInfo) => {
  await expectSharedNavigation(page);
  await expect(
    page.getByRole("columnheader", { name: "标签", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".file-view-controls")).toHaveCount(0);
  await expect(page.locator(".library-page")).toBeHidden();
  for (const width of [1838, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await page.locator(".search-tags summary").click();
    await expect(
      page.getByRole("listbox", { name: "全部标签", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("listbox", { name: "任一标签", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("listbox", { name: "排除标签", exact: true }),
    ).toBeVisible();
    await expect(page.locator(".paths-table")).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: testInfo.outputPath(`directory-tags-${width}.png`),
    });
    await page.locator(".search-tags summary").click();
  }
});

test("File tag editing, combined search, and history navigation use the same directory table", async ({
  appPage: page,
}, testInfo) => {
  const tagName = `目录标签-${randomUUID().slice(0, 8)}`;
  const waitForJsonResponse = await observeJsonResponses(page);
  await page.locator('.xcss-header-navigation a[href="#tags"]').click();
  await page
    .getByRole("textbox", { name: "标签名称", exact: true })
    .fill(tagName);
  const [{ payload: created }] = await Promise.all([
    waitForJsonResponse(
      (response) =>
        response.request().method() === "POST" &&
        new URL(response.url()).pathname === "/api/v1/file-tags/tags",
    ),
    page.getByRole("button", { name: "创建标签", exact: true }).click(),
  ]);
  await expect(
    page.getByRole("table", { name: "标签列表", exact: true }),
  ).toContainText(tagName);
  await page.locator('.xcss-header-navigation a[href="#files"]').click();
  const row = page.locator(".paths-table tbody tr").filter({
    has: page.locator(".cell-name a").filter({ hasText: /^download-me\.txt$/ }),
  });
  await page.locator('[data-file-action="tags"]').click();
  await row.locator(".cell-name a").click();
  const dialog = page.getByRole("region", { name: "文件标签", exact: true });
  await expect(dialog.locator(".file-tag-target")).toHaveText("download-me.txt");
  await ensureIndexed(dialog);
  const [saved] = await Promise.all([
    page.waitForResponse(
      (response) =>
        response.request().method() === "POST" &&
        new URL(response.url()).pathname === "/api/v1/file-tags/file-tags",
    ),
    dialog.getByRole("button", { name: `添加标签 ${tagName}`, exact: true }).click(),
  ]);
  expect(saved.status()).toBe(204);
  expect(saved.request().postDataJSON().tag_ids).toEqual([created.id]);
  await expect(dialog).toContainText(tagName);
  await page.locator('[data-file-action="tags"]').click();
  await expect(row.locator(".cell-tags .file-tag-label")).toHaveText(tagName);
  await expect(row.locator(".cell-actions")).toHaveCount(0);
  const marker = randomUUID(),
    documents = [];
  await page.evaluate((value) => {
    window.xczsSearchMarker = value;
  }, marker);
  page.on("request", (request) => {
    if (request.isNavigationRequest() && request.frame() === page.mainFrame())
      documents.push(request.url());
  });
  await page
    .getByRole("textbox", { name: "搜索文件或文件夹", exact: true })
    .fill("download-me.txt");
  await page.locator(".search-tags summary").click();
  await page
    .getByRole("listbox", { name: "全部标签", exact: true })
    .selectOption(String(created.id));
  const listed = page.waitForResponse((response) => {
    const url = new URL(response.url());
    return (
      url.pathname === "/__xczs__/api/list" &&
      url.searchParams.get("q") === "download-me.txt" &&
      url.searchParams.get("all") === String(created.id)
    );
  });
  await page
    .locator(".search-tag-options")
    .getByRole("button", { name: "搜索", exact: true })
    .click();
  expect((await listed).status()).toBe(200);
  await expect(row).toBeVisible();
  await expect(page.locator(".paths-table tbody tr")).toHaveCount(1);
  await expect(row.locator(".cell-tags")).toContainText(tagName);
  await page.screenshot({
    path: testInfo.outputPath("filtered-directory-with-tags.png"),
  });
  await page.locator(".search-tags summary").click();
  await page
    .getByRole("listbox", { name: "全部标签", exact: true })
    .selectOption([]);
  await page
    .getByRole("listbox", { name: "任一标签", exact: true })
    .selectOption(String(created.id));
  await page
    .locator(".search-tag-options")
    .getByRole("button", { name: "搜索", exact: true })
    .click();
  await expect(page).toHaveURL(/any=/);
  await expect(row).toBeVisible();
  await page.locator(".search-tags summary").click();
  await page
    .getByRole("listbox", { name: "任一标签", exact: true })
    .selectOption([]);
  await page
    .getByRole("listbox", { name: "排除标签", exact: true })
    .selectOption(String(created.id));
  await page
    .locator(".search-tag-options")
    .getByRole("button", { name: "搜索", exact: true })
    .click();
  await expect(page.locator(".empty-folder")).toHaveText("没有搜索结果");
  await page.goBack();
  await expect(page).toHaveURL(/any=/);
  await expect(row).toBeVisible();
  await page.goForward();
  await expect(page.locator(".empty-folder")).toHaveText("没有搜索结果");
  await page.locator(".search-tags summary").click();
  await page.getByRole("button", { name: "清除标签筛选", exact: true }).click();
  await expect(row).toBeVisible();
  expect(await page.evaluate(() => window.xczsSearchMarker)).toBe(marker);
  expect(documents).toEqual([]);
  await page.locator('[data-file-action="tags"]').click();
  await row.locator(".cell-name a").click();
  await dialog
    .getByRole("button", { name: `移除标签 ${tagName}`, exact: true })
    .click();
  await expect(
    dialog.getByRole("button", { name: `移除标签 ${tagName}`, exact: true }),
  ).toHaveCount(0);
  await page.locator('[data-file-action="tags"]').click();
  await expect(row.locator(".cell-tags .file-tag-label")).toHaveCount(0);
  const downloaded = page.waitForEvent("download");
  await page.locator('[data-file-action="download"]').click();
  await row.locator(".cell-name a").click();
  expect((await downloaded).suggestedFilename()).toBe("download-me.txt");
});

test("Adding multiple tags consecutively and switching files modifies only the current file", async ({ appPage: page }) => {
  const names = ["多个标签甲-", "多个标签乙-"].map(prefix => prefix + randomUUID().slice(0, 8));
  await page.locator('.xcss-header-navigation a[href="#tags"]').click();
  for (const name of names) {
    await page.getByRole("textbox", { name: "标签名称", exact: true }).fill(name);
    await page.getByRole("button", { name: "创建标签", exact: true }).click();
    await expect(page.getByRole("table", { name: "标签列表", exact: true })).toContainText(name);
  }
  await page.locator('.xcss-header-navigation a[href="#files"]').click();
  await page.locator('[data-file-action="tags"]').click();
  const first = page.getByRole("link", { name: "download-me.txt", exact: true });
  await first.click();
  const bar = page.getByRole("region", { name: "文件标签", exact: true });
  await ensureIndexed(bar);
  for (const name of names) {
    await bar.getByRole("button", { name: `添加标签 ${name}`, exact: true }).click();
    await expect(bar.getByRole("button", { name: `移除标签 ${name}`, exact: true })).toHaveAttribute("aria-pressed", "true");
  }
  const firstRow = first.locator("xpath=ancestor::tr");
  for (const name of names) await expect(firstRow.locator(".cell-tags")).toContainText(name);
  for (const width of [1280, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
  await page.getByRole("link", { name: "rename-me.txt", exact: true }).click();
  await expect(bar.locator(".file-tag-target")).toHaveText("rename-me.txt");
  await ensureIndexed(bar);
  const add = bar.getByRole("button", { name: `添加标签 ${names[0]}`, exact: true });
  await expect(add).toHaveAttribute("aria-pressed", "false");
  await add.click();
  const remove = bar.getByRole("button", { name: `移除标签 ${names[0]}`, exact: true });
  await expect(remove).toBeEnabled();
  await remove.click();
  await expect(remove).toHaveCount(0);
  const secondRow = page.getByRole("link", { name: "rename-me.txt", exact: true }).locator("xpath=ancestor::tr");
  await expect(secondRow.locator(".file-tag-label")).toHaveCount(0);
  for (const name of names) await expect(firstRow.locator(".cell-tags")).toContainText(name);
  await page.keyboard.press("Escape");
  await expect(bar).toHaveCount(0);
  await expect(page.locator(".is-tag-selected")).toHaveCount(0);
});

test("Tag rows in subdirectories and for special filenames still target the original file in the current directory", async ({ appPage: page }) => {
  const directory = new URL(page.url()).pathname;
  const special = "special & # + 中文.txt";
  await page.locator('[data-file-action="tags"]').click();
  await page.getByRole("link", { name: "existing-folder", exact: true }).click();
  expect(new URL(page.url()).pathname).toBe(directory);
  await expect(page.locator("#file-action-hint")).toContainText("文件夹不支持标签");
  await page.getByRole("link", { name: special, exact: true }).click();
  const bar = page.getByRole("region", { name: "文件标签", exact: true });
  await expect(bar.locator(".file-tag-target")).toHaveText(special);
  await ensureIndexed(bar);
  await page.keyboard.press("Escape");
  await expect(bar).toHaveCount(0);
  await page.getByRole("link", { name: "existing-folder", exact: true }).click();
  await expect(page.getByRole("link", { name: "nested.txt", exact: true })).toBeVisible();
  await page.locator('[data-file-action="tags"]').click();
  const request = page.waitForResponse(response => {
    const url = new URL(response.url());
    return url.pathname === "/api/v1/file-tags/file" && url.searchParams.get("path")?.endsWith("/existing-folder/nested.txt");
  });
  await page.getByRole("link", { name: "nested.txt", exact: true }).click();
  expect((await request).status()).toBe(200);
  await expect(bar.locator(".file-tag-target")).toHaveText("nested.txt");
  expect(new URL(page.url()).pathname).toBe(directory + "existing-folder/");
});

test("An open tag filter native fallback refreshes and selects newly created tags", async ({ appPage: page }) => {
  const name = `候选刷新-${randomUUID().slice(0, 8)}`;
  const panel = page.locator('.search-tag-options');
  const picker = page.getByRole("listbox", { name: "全部标签", exact: true });
  await page.locator(".search-tags summary").click();
  await expect(picker).toBeVisible();
  await page.locator('.xcss-header-navigation a[href="#tags"]').click();
  await page.getByRole("textbox", { name: "标签名称", exact: true }).fill(name);
  await page.getByRole("button", { name: "创建标签", exact: true }).click();
  await expect(page.getByRole("table", { name: "标签列表", exact: true })).toContainText(name);
  await page.locator('.xcss-header-navigation a[href="#files"]').click();
  // Menu changes preserve this open details element. No toggle or reload should
  // be needed before a newly created tag can become a search condition.
  await expect(picker).toBeVisible();
  // Apply CSSOM to the current panel after navigation; keep the production CSP intact.
  await panel.evaluate(node => node.style.setProperty('box-shadow', 'inset 0 0 2px black', 'important'));
  await expect(panel).toHaveAttribute('data-xcss-server-corner-state', 'fallback');
  await expect(panel).toHaveAttribute('data-xcss-server-corner-reason', 'inset-or-unsupported-shadow');
  await expect(panel).toBeVisible();
  const option = picker.locator("option").filter({ hasText: name });
  await expect(option).toHaveCount(1);
  await picker.selectOption({ label: name });
  await expect(picker.locator('option:checked')).toHaveText(name);
  await panel.evaluate(node => node.style.removeProperty('box-shadow'));
  await expect(panel).toHaveAttribute('data-xcss-server-corner-state', 'clip');
  await expect(panel).toBeVisible();
  await page.locator(".search-tag-options").getByRole("button", { name: "搜索", exact: true }).click();
  await expect(page.locator(".empty-folder")).toHaveText("没有搜索结果");
});
