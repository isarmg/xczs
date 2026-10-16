const { readFileSync, readdirSync } = require("node:fs");
const { join, resolve } = require("node:path");
const AxeBuilder = require("@axe-core/playwright").default;
const {
  actionDialog,
  chooseFileAction,
  expect,
  login,
  rowByName,
  test,
} = require("./fixtures");

const axeTags = [
  "wcag2a",
  "wcag2aa",
  "wcag21a",
  "wcag21aa",
  "wcag22a",
  "wcag22aa",
];

function axe(page) {
  return new AxeBuilder({ page }).withTags(axeTags);
}

function walkJavaScript(root, extension = /\.tsx?$/u) {
  return readdirSync(root, { withFileTypes: true }).flatMap(entry => {
    const path = join(root, entry.name);
    if (entry.isDirectory()) return walkJavaScript(path, extension);
    return entry.isFile() && extension.test(entry.name) ? [path] : [];
  });
}

function fieldPaint(control) {
  return control.evaluate(element => {
    const style = getComputedStyle(element);
    return {
      borders: ["Top", "Right", "Bottom", "Left"].map(side => [
        style[`border${side}Width`], style[`border${side}Style`], style[`border${side}Color`],
      ]),
      shadow: style.boxShadow,
      outline: style.outlineStyle,
    };
  });
}

test("Primary file controls use native semantics and keyboard interaction", async ({ appPage: page }) => {
  const root = page.getByRole("link", { name: "Root", exact: true });
  await expect(root).toBeVisible();
  await expect(root).toHaveAttribute("href", "/");
  await expect(root).toHaveAttribute("title", "Root");
  await expect(root).toHaveText("");
  await expect(root.locator("xpath=ancestor::main")).toHaveCount(1);
  for (const selector of [".upload-file", ".upload-folder", ".new-folder", ".new-file", ".searchbar"]) {
    await expect(page.locator(selector).locator("xpath=ancestor::main")).toHaveCount(1);
  }
  const rootIcon = root.locator("svg");
  await expect(rootIcon).toHaveCount(1);
  await expect(rootIcon).toHaveAttribute("aria-hidden", "true");

  for (
    const name of [
      "Upload files",
      "Upload folder",
      "New folder",
      "New empty file",
      "Sign out",
    ]
  ) {
    const control = page.getByRole("button", { name, exact: true });
    await expect(control).toBeVisible();
    expect(await control.evaluate(element => element.tagName)).toBe("BUTTON");
  }

  const fileChooserPromise = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "Upload files", exact: true }).focus();
  await page.keyboard.press("Enter");
  const chooser = await fileChooserPromise;
  await chooser.setFiles([]);
  await expect(
    page.getByRole("button", { name: "Upload files", exact: true }),
  ).toBeFocused();

  const newFolder = page.getByRole("button", { name: "New folder" });
  await newFolder.focus();
  await page.keyboard.press("Space");
  const folderName = page.locator(".inline-name-input");
  await expect(folderName).toHaveCount(1);
  await expect(folderName).toBeFocused();
  await expect(folderName).toHaveValue("newfolder");
  await expect(folderName).toHaveAttribute("aria-label", /newfolder/i);
  await page.keyboard.press("Escape");
  await expect(folderName).toHaveCount(0);
  await expect(
    page.getByRole("link", { name: "newfolder", exact: true }),
  ).toBeVisible();
  await expect(newFolder).toBeFocused();

  const newFile = page.getByRole("button", { name: "New empty file" });
  await newFile.focus();
  await page.keyboard.press("Enter");
  const fileName = page.locator(".inline-name-input");
  await expect(fileName).toBeFocused();
  await expect(fileName).toHaveValue("newfile");
  await expect(fileName).toHaveAttribute("aria-label", /newfile/i);
  await page.keyboard.press("Escape");
  await expect(fileName).toHaveCount(0);
  await expect(newFile).toBeFocused();

  const row = rowByName(page, "download-me.txt");
  const rename = page.locator('[data-file-action="rename"]');
  const move = page.locator('[data-file-action="move"]');
  const remove = page.locator('[data-file-action="delete"]');
  expect(await rename.evaluate(element => element.tagName)).toBe("BUTTON");
  expect(await move.evaluate(element => element.tagName)).toBe("BUTTON");
  expect(await remove.evaluate(element => element.tagName)).toBe("BUTTON");
  await rename.focus();
  await page.keyboard.press("Enter");
  await row.locator(".cell-name a").focus();
  await page.keyboard.press("Enter");
  const renameInput = page.locator(".inline-name-input");
  await expect(renameInput).toBeFocused();
  await expect(renameInput).toHaveValue("download-me.txt");
  await expect(renameInput).toHaveAttribute("aria-label", /download-me\.txt/i);
  await page.keyboard.press("Escape");
  await expect(renameInput).toHaveCount(0);
  await expect(rename).toBeFocused();
  await remove.focus();
  await page.keyboard.press("Enter");
  await row.locator(".cell-name a").focus();
  await page.keyboard.press("Space");
  const deleteDialog = actionDialog(page, "Delete item");
  await expect(deleteDialog).toContainText(
    'Delete "download-me.txt"? This action cannot be undone.',
  );
  await expect(deleteDialog).toHaveAttribute(
    "aria-describedby",
    "action-dialog-message",
  );
  await expect(deleteDialog).toHaveAccessibleDescription(
    'Delete "download-me.txt"? This action cannot be undone.',
  );
  const confirmDelete = deleteDialog.getByRole("button", { name: "Delete" });
  await expect(confirmDelete).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(remove).toBeFocused();
  await expect(page.locator(".upload-queue-message")).toHaveAttribute(
    "aria-live",
    "assertive",
  );

  await expect(page.locator("#file")).toHaveAttribute("tabindex", "-1");
  await expect(page.locator("#folder")).toHaveAttribute("tabindex", "-1");
  const search = page.getByLabel("Search files or folders");
  const searchbar = page.locator(".searchbar");
  const beforeFocus = await Promise.all([search, searchbar].map(fieldPaint));
  await search.focus();
  await expect(search).toBeFocused();
  const afterFocus = await Promise.all([search, searchbar].map(fieldPaint));
  for (let index = 0; index < afterFocus.length; index++) {
    expect(afterFocus[index].outline).toBe("none");
    expect(afterFocus[index].borders).toEqual(beforeFocus[index].borders);
    expect(afterFocus[index].shadow).toBe(beforeFocus[index].shadow);
  }

  for (const control of [
    page.getByRole("button", { name: "Upload files", exact: true }),
    rename,
    move,
    remove,
  ]) {
    const box = await control.boundingBox();
    expect(box.width).toBeGreaterThan(23.9);
    expect(box.height).toBeGreaterThan(23.9);
  }
});

test("The secondary menu has five mode buttons and the directory table has no action column", async ({ appPage: page }) => {
  await expect(page.locator(".paths-table .cell-actions")).toHaveCount(0);
  const actions = page.locator(".xczs-file-actions [data-file-action]");
  await expect(actions).toHaveCount(5);
  for (const width of [1280, 320]) {
    await page.setViewportSize({ width, height: 800 });
    for (const button of await actions.all()) {
      await expect(button).toBeVisible();
      await expect(button).toHaveAttribute("aria-pressed", "false");
      const box = await button.boundingBox();
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(width);
    }
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
});

test("A 1280px desktop at 400% zoom reflows within 320 CSS pixels", async ({
  appPage: page,
}) => {
  await page.setViewportSize({ width: 320, height: 800 });
  await expect(page.getByRole("button", { name: "Upload files" })).toBeVisible();
  await expect(page.getByLabel("Search files or folders")).toBeVisible();
  await expect(page.getByRole("button", { name: "Sign out" })).toBeVisible();

  const row = rowByName(page, "download-me.txt");
  await expect(row.locator(".cell-mtime")).toBeVisible();
  await expect(row.locator(".cell-size")).toBeVisible();
  for (const action of ["rename", "move", "delete"])
    await expect(page.locator(`[data-file-action="${action}"]`)).toBeVisible();
  await chooseFileAction(page, "rename", "download-me.txt");
  const inlineEditor = page.locator(".inline-name-input");
  await expect(inlineEditor).toBeFocused();
  await expect(page.locator(".inline-name-marker")).toHaveCount(0);
  await inlineEditor.fill("w".repeat(255));
  const editorLayout = await inlineEditor.evaluate(element => {
    const rect = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return {
      clientWidth: document.documentElement.clientWidth,
      documentScrollWidth: document.documentElement.scrollWidth,
      inputClientWidth: element.clientWidth,
      inputScrollWidth: element.scrollWidth,
      left: rect.left,
      right: rect.right,
      plainTextOnly: style.backgroundColor === "rgba(0, 0, 0, 0)" &&
        Number.parseFloat(style.borderTopWidth) === 0 &&
        Number.parseFloat(style.borderRightWidth) === 0 &&
        Number.parseFloat(style.borderLeftWidth) === 0 &&
        style.borderBottomStyle === "dashed" && Number.parseFloat(style.borderBottomWidth) >= 1 &&
        style.borderRadius === "0px" &&
        style.outlineStyle === "none" &&
        style.boxShadow === "none",
    };
  });
  expect(editorLayout.documentScrollWidth).toBeLessThanOrEqual(editorLayout.clientWidth);
  expect(editorLayout.left).toBeGreaterThanOrEqual(0);
  expect(editorLayout.right).toBeLessThanOrEqual(editorLayout.clientWidth);
  expect(editorLayout.inputScrollWidth).toBeGreaterThan(editorLayout.inputClientWidth);
  expect(editorLayout.plainTextOnly).toBe(true);
  await inlineEditor.press("Escape");

  const layout = await page.evaluate(() => {
    const actionCell = document.querySelector(".xczs-file-actions");
    const rect = actionCell.getBoundingClientRect();
    return {
      clientWidth: document.documentElement.clientWidth,
      scrollWidth: document.documentElement.scrollWidth,
      actionLeft: rect.left,
      actionRight: rect.right,
    };
  });
  expect(layout.clientWidth).toBe(320);
  expect(layout.scrollWidth).toBeLessThanOrEqual(layout.clientWidth);
  expect(layout.actionLeft).toBeGreaterThanOrEqual(0);
  expect(layout.actionRight).toBeLessThanOrEqual(layout.clientWidth);
});

test("Forced colors preserve inline editor focus and dialog semantics", async ({
  appPage: page,
}) => {
  await page.emulateMedia({ forcedColors: "active" });
  expect(
    await page.evaluate(() => matchMedia("(forced-colors: active)").matches),
  ).toBe(true);

  const trigger = page.getByRole("button", { name: "New empty file" });
  await trigger.focus();
  await trigger.click();
  const input = page.locator(".inline-name-input");
  await expect(input).toBeFocused();
  expect(
    await input.evaluate(element => {
      const style = getComputedStyle(element);
      return style.borderTopStyle === "none" &&
        style.borderRightStyle === "none" &&
        style.borderLeftStyle === "none" &&
        style.borderBottomStyle === "dashed" && Number.parseFloat(style.borderBottomWidth) === 1 &&
        style.outlineStyle === "none" &&
        style.boxShadow === "none" &&
        style.caretColor !== "rgba(0, 0, 0, 0)";
    }),
  ).toBe(true);
  await expect(page.locator(".inline-name-marker")).toHaveCount(0);

  await page.keyboard.press("Escape");
  await expect(input).toHaveCount(0);
  await expect(trigger).toBeFocused();
});

test("The login page passes the axe WCAG A/AA scan", async ({ axePage: page }) => {
  await page.goto("/__xczs__/login");
  const results = await axe(page).analyze();
  expect(results.violations).toEqual([]);
});

test("File pages, inline editors, and operation dialogs pass the axe WCAG A/AA scan", async ({
  axePage: page,
}, testInfo) => {
  // This case deliberately runs three axe analyses. On the fully-parallel
  // Chromium/Firefox matrix, 30 seconds is not a reliable combined CPU budget.
  test.setTimeout(60_000);
  await login(page, testInfo.parallelIndex);
  const pageResults = await axe(page).analyze();
  expect(pageResults.violations).toEqual([]);

  await page.getByRole("button", { name: "New folder" }).click();
  const input = page.locator(".inline-name-input");
  await expect(input).toBeVisible();
  const editorResults = await axe(page).include(".is-renaming").analyze();
  expect(editorResults.violations).toEqual([]);
  await page.keyboard.press("Escape");

  await chooseFileAction(page, "delete", "download-me.txt");
  const dialog = actionDialog(page, "Delete item");
  await expect(dialog).toBeVisible();
  // The page itself was scanned above. Scope the modal-state scan to the open
  // dialog so axe does not rescan the inert page subtree under CPU contention.
  const dialogResults = await axe(page).include(".action-dialog").analyze();
  expect(dialogResults.violations).toEqual([]);
  await page.keyboard.press("Escape");
});

test("Production frontend source has no dynamic HTML injection or native browser modal calls", async () => {
  const files = [
    resolve(__dirname, "../../web/leptos-index.js"),
    resolve(__dirname, "../../web/leptos-login.js"),
    resolve(__dirname, "../../web/leptos-platform.js"),
    ...walkJavaScript(resolve(__dirname, "../../web/leptos/src"), /\.rs$/u),
    ...walkJavaScript(resolve(__dirname, "../../web/leptos/vendor/xcss-web/src"), /\.rs$/u),
  ];
  const forbidden = /\b(?:innerHTML|outerHTML|insertAdjacentHTML|document\.write|DOMParser)\b/;
  const nativeModal = /(?<![.\w])(?:alert|confirm|prompt)\s*\(|\b(?:globalThis|self|window)\s*\.\s*(?:alert|confirm|prompt)\s*\(/u;
  for (const file of files) {
    const source = readFileSync(file, "utf8");
    expect(source, file).not.toMatch(forbidden);
    if (file.endsWith(".rs")) {
      expect(source, file).not.toMatch(/\b(?:set_inner_html|set_outer_html|insert_adjacent_html|write_with_str|alert_with_message|confirm_with_message|prompt_with_message)\b/u);
    } else {
      expect(source, file).not.toMatch(nativeModal);
    }
  }
});

test("Chinese and English interfaces stay consistent while language switching preserves sessions, file types, and user filenames", async ({ appPage: page }) => {
  const switchToChinese = page.getByRole("button", { name: "Switch to Chinese", exact: true });
  await switchToChinese.click();
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
  await expect(page.getByRole("table", { name: "文件列表", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "新建文件夹", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "退出", exact: true })).toBeVisible();
  await expect(page.locator(".list-status")).toContainText("已加载");
  await expect(page.locator(".paths-table thead")).not.toContainText(/Name|Modified|Size/);
  await expect(page.getByRole("link", { name: "existing-folder", exact: true })).toBeVisible();
  await expect(page.getByRole("link", { name: "special & # + 中文.txt", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "切换为英文", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await expect(page.getByRole("table", { name: "File list", exact: true })).toBeVisible();
  await expect(page.locator(".paths-table thead")).not.toContainText(/\p{Script=Han}/u);
  const url = new URL(page.url()); url.searchParams.delete("lang");
  await page.goto(url.href); await expect(page.locator("html")).toHaveAttribute("lang", "en");
});
