const { randomUUID } = require("node:crypto");
const {
  test, expect, rowByName, chooseFileAction, actionDialog,
  currentDirectoryPath,
} = require("./fixtures");

test("Choosing an operation before a file, cancelling the mode, and switching menus do not mutate files", async ({ appPage: page }) => {
  const changes = [];
  page.on("request", request => {
    if (["DELETE", "POST"].includes(request.method()) && /\/(__xczs__\/api\/(move|rename)|delete-me\.txt)$/.test(new URL(request.url()).pathname))
      changes.push(request.url());
  });
  await expect(page.locator(".paths-table .cell-actions")).toHaveCount(0);
  const rename = page.locator('[data-file-action="rename"]');
  await rename.click();
  await expect(rename).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".inline-name-input")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(rename).toHaveAttribute("aria-pressed", "false");
  await chooseFileAction(page, "delete", "delete-me.txt");
  await expect(actionDialog(page, "Delete item")).toBeVisible();
  expect(changes).toEqual([]);
  await page.keyboard.press("Escape");
  await expect(rowByName(page, "delete-me.txt")).toBeVisible();
  const move = page.locator('[data-file-action="move"]');
  await move.click();
  await page.locator('.xcss-header-navigation a[href="#status"]').click();
  await page.locator('.xcss-header-navigation a[href="#files"]').click();
  await expect(move).toHaveAttribute("aria-pressed", "false");
  expect(changes).toEqual([]);
});

test("Secondary-menu download accepts files only and does not navigate when a directory is selected", async ({ appPage: page }) => {
  const directory = page.url();
  const mode = page.locator('[data-file-action="download"]');
  await mode.click();
  await rowByName(page, "existing-folder").locator(".cell-icon").click();
  expect(page.url()).toBe(directory);
  await expect(page.locator("#file-action-hint")).toContainText("Folders cannot be downloaded");
  await expect(mode).toHaveAttribute("aria-pressed", "true");
  const downloaded = page.waitForEvent("download");
  await rowByName(page, "download-me.txt").locator(".cell-name a").click();
  expect((await downloaded).suggestedFilename()).toBe("download-me.txt");
  await expect(mode).toHaveAttribute("aria-pressed", "false");
});

test("Clicking a directory row starts inline rename, then the menu moves it to the destination directory", async ({ appPage: page }) => {
  const directory = currentDirectoryPath(page);
  const name = `renamed-folder-${randomUUID().slice(0, 8)}`;
  const mode = page.locator('[data-file-action="rename"]');
  await mode.click();
  await rowByName(page, "existing-folder").locator(".cell-icon").click();
  const editor = page.locator(".inline-name-input");
  await expect(editor).toHaveValue("existing-folder");
  await editor.fill(name);
  await editor.press("Enter");
  await expect(rowByName(page, name)).toBeVisible();
  await chooseFileAction(page, "move", name);
  const dialog = actionDialog(page, "Move item");
  await dialog.getByRole("textbox", { name: "Destination folder" }).fill("/");
  await dialog.getByRole("button", { name: "Move", exact: true }).click();
  await expect.poll(() => currentDirectoryPath(page)).toBe("/");
  await expect(rowByName(page, name)).toBeVisible();
  await chooseFileAction(page, "delete", name);
  await actionDialog(page, "Delete item").getByRole("button", { name: "Delete", exact: true }).click();
  await expect(rowByName(page, name)).toHaveCount(0);
  await page.goto(directory + "/");
  await expect(rowByName(page, "download-me.txt")).toBeVisible();
});
