const { randomUUID } = require("node:crypto");
const { test, expect, selectFiles } = require("./fixtures.js");

test("Menus and browser history preserve the same document, directory listing, and input state", async ({
  appPage: page,
}) => {
  const originalUrl = new URL(page.url());
  const marker = randomUUID();
  await page.evaluate((value) => {
    window.xczsNavigationMarker = value;
  }, marker);
  const navigations = [];
  page.on("request", (request) => {
    if (request.isNavigationRequest() && request.frame() === page.mainFrame())
      navigations.push(request.url());
  });
  const row = await page
    .locator(".paths-table tbody tr")
    .first()
    .elementHandle();
  await page
    .getByRole("textbox", { name: "Search files or folders", exact: true })
    .fill("retained file search");
  const menu = page.getByRole("navigation", { name: "Main navigation" });
  await page.locator(".search-tags summary").click();
  await expect(
    page.getByRole("table", { name: "File list", exact: true }),
  ).toBeVisible();
  await expect(
    menu.getByRole("link", { name: "Files", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await expect(
    page.getByRole("listbox", { name: "All tags", exact: true }),
  ).toBeVisible();
  await menu.getByRole("link", { name: "Manage tags", exact: true }).click();
  await page
    .getByRole("textbox", { name: "Tag name", exact: true })
    .fill("retained tag draft");
  await menu.getByRole("link", { name: "Service status", exact: true }).click();
  await expect(page.locator(".library-status")).toBeVisible();
  await menu.getByRole("link", { name: "Files", exact: true }).click();
  await expect(page.locator(".index-page")).toBeVisible();
  await expect(
    page.getByRole("textbox", { name: "Search files or folders", exact: true }),
  ).toHaveValue("retained file search");
  expect(
    await row.evaluate(
      (element) =>
        element.isConnected &&
        element === document.querySelector(".paths-table tbody tr"),
    ),
  ).toBe(true);
  await page.goBack();
  await expect(page).toHaveURL(/#status$/);
  await expect(page.locator(".library-status")).toBeVisible();
  await page.goForward();
  await expect(page).toHaveURL(/#files$/);
  await expect(page.locator(".index-page")).toBeVisible();
  await menu.getByRole("link", { name: "Manage tags", exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "Tag name", exact: true }),
  ).toHaveValue("retained tag draft");
  await menu.getByRole("link", { name: "Files", exact: true }).click();
  await page.locator(".search-tags summary").click();
  await expect(
    page.getByRole("textbox", { name: "Search files or folders", exact: true }),
  ).toHaveValue("retained file search");
  await menu.getByRole("link", { name: "Files", exact: true }).click();
  expect(new URL(page.url()).pathname).toBe(originalUrl.pathname);
  expect(new URL(page.url()).search).toBe(originalUrl.search);
  expect(await page.evaluate(() => window.xczsNavigationMarker)).toBe(marker);
  expect(navigations).toEqual([]);
  await expect(page.locator("#xczs-root > header")).toHaveCount(1);
});

test("Switching every menu during upload neither unmounts tasks nor prompts to leave the page", async ({
  appPage: page,
}) => {
  const originalPath = new URL(page.url()).pathname;
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  let started;
  const reached = new Promise((resolve) => {
    started = resolve;
  });
  const requests = [];
  const dialogs = [];
  page.on("dialog", (dialog) => {
    dialogs.push(dialog.type());
    void dialog.dismiss();
  });
  await page.route("**/navigation-upload.txt", async (route) => {
    if (route.request().method() === "PUT") {
      requests.push(route.request().url());
      started();
      await gate;
    }
    await route.continue();
  });
  try {
    await selectFiles(page, "#file", [
      {
        name: "navigation-upload.txt",
        mimeType: "text/plain",
        buffer: Buffer.from("Upload survives every menu"),
      },
    ]);
    await reached;
    const row = await page.locator(".upload-status").elementHandle();
    const menu = page.getByRole("navigation", { name: "Main navigation" });
    await page.locator(".search-tags summary").click();
    await expect(page.locator(".upload-status")).toBeVisible();
    await expect(page.locator(".paths-table")).toBeVisible();
    await page.locator(".search-tags summary").click();
    await expect(page.locator(".paths-table")).toBeVisible();
    await page
      .getByRole("textbox", { name: "Search files or folders", exact: true })
      .fill("download-me.txt");
    await page
      .getByRole("textbox", { name: "Search files or folders", exact: true })
      .press("Enter");
    await expect(page.locator(".upload-status")).toBeVisible();
    for (const [label, hash] of [
      ["Manage tags", "#tags"],
      ["Service status", "#status"],
      ["Files", "#files"],
    ]) {
      await menu.getByRole("link", { name: label, exact: true }).click();
      await expect.poll(() => new URL(page.url()).hash).toBe(hash);
      expect(
        await row.evaluate(
          (element) =>
            element.isConnected &&
            element === document.querySelector(".upload-status"),
        ),
      ).toBe(true);
    }
    expect(requests).toHaveLength(1);
    expect(new URL(requests[0]).pathname).toBe(
      originalPath + "navigation-upload.txt",
    );
    expect(dialogs).toEqual([]);
  } finally {
    release();
  }
  await expect(page.locator(".upload-status")).toHaveAttribute(
    "aria-label",
    "navigation-upload.txt: upload complete",
  );
  expect(requests).toHaveLength(1);
});

test("Current menu deep links enter the directory application", async ({ appPage: page }) => {
  const directory = new URL(page.url()).pathname;
  for (const [view, selector] of [
    ["files", ".paths-table"],
    ["tags", ".library-tag-form"],
    ["status", ".library-status"],
  ]) {
    await page.goto(`${directory}#${view}`);
    await expect(page).toHaveURL(new RegExp(`${directory}#${view}$`));
    await expect(page.locator(selector)).toBeVisible();
    await expect(page.locator("#xczs-root > header")).toHaveCount(1);
    await expect(
      page.locator(".xcss-header-navigation a[aria-current=page]"),
    ).toHaveAttribute("href", `#${view}`);
  }
  await page.goto(directory + "#tags");
  await expect(page.locator(".library-tag-form")).toBeVisible();
  await page
    .getByRole("navigation", { name: "Main navigation" })
    .getByRole("link", { name: "Files", exact: true })
    .click();
  await expect(
    page.getByRole("link", { name: "existing-folder", exact: true }),
  ).toBeVisible();
  expect(new URL(page.url()).pathname).toBe(directory);
});

test("The current files deep link retains the directory and allows creating files", async ({
  appPage: page,
}) => {
  const directory = new URL(page.url()).pathname;
  await page.goto(directory + "#files");
  await expect(page.locator(".search-tags summary")).toBeVisible();
  await page
    .getByRole("button", { name: "New empty file", exact: true })
    .click();
  await expect(page).toHaveURL(/#files$/);
  await expect(page.locator(".inline-name-input")).toBeVisible();
  await page.locator(".inline-name-input").press("Escape");
  await expect(
    page.getByRole("link", { name: "newfile", exact: true }),
  ).toBeVisible();
  expect(new URL(page.url()).pathname).toBe(directory);
});

test("Unknown menu links show an error without rewriting the URL or reloading controllers", async ({ appPage: page }) => {
  const directory = new URL(page.url()).pathname;
  const row = await page.locator(".paths-table tbody tr").first().elementHandle();
  for (const hash of ["#missing", "#tag-files", "#files/tags"]) {
    await page.evaluate(value => { window.location.hash = value; }, hash);
    await expect(page.getByRole("alert")).toContainText("Page not found. Choose a menu above.");
    await expect(page.locator(".index-page")).toBeHidden();
    await expect(page.locator(".xcss-header-navigation a[aria-current=page]")).toHaveCount(0);
    expect(new URL(page.url()).hash).toBe(hash);
    await page.getByRole("navigation", { name: "Main navigation" }).getByRole("link", { name: "Files", exact: true }).click();
    await expect(page.locator(".index-page")).toBeVisible();
    expect(await row.evaluate(element => element.isConnected)).toBe(true);
    expect(new URL(page.url()).pathname).toBe(directory);
  }
});

test("Unified application tag requests return to login when the session expires", async ({ appPage: page }) => {
  await page.context().clearCookies();
  await page
    .getByRole("navigation", { name: "Main navigation" })
    .getByRole("link", { name: "Service status", exact: true })
    .click();
  await expect(page).toHaveURL(/\/__xczs__\/login$/);
  await expect(
    page.getByRole("button", { name: "Sign in", exact: true }),
  ).toBeVisible();
});
