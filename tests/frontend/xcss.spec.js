const { readFileSync } = require("node:fs");
const { createHash } = require("node:crypto");
const AxeBuilder = require("@axe-core/playwright").default;
const { test, expect, pageData, login, selectFiles } = require("./fixtures.js");

test("Leptos frontend uses embedded fonts, licenses, and session recovery supplied by xcss", async ({ appPage: page }) => {
  await pageData(page);
  const platformCss = page.locator('link[rel="stylesheet"][href$="/dist/platform.css"]');
  const prefix = new URL("./", await platformCss.evaluate(link => link.href));
  const boot = page.locator('link[rel="stylesheet"][href$="/dist/boot.css"]');
  await expect(boot).toHaveCount(1);
  const bootResponse = await page.context().request.get(await boot.evaluate(link => link.href));
  expect(bootResponse.status()).toBe(200);
  expect(await bootResponse.body()).toEqual(readFileSync(require.resolve("@xcss/web/web-fonts/boot.css")));
  expect(await page.locator("head").evaluate(head => head.querySelector("style") === null)).toBe(true);
  const licenses = await page.evaluate(async url => {
    const platform = await import(url);
    return { latin: platform.fontLicenseUrl, cjk: platform.cjkFontLicenseUrl };
  }, new URL("platform.js", prefix).href);
  const provenance = require("../../node_modules/@xcss/web/dist/web-fonts/provenance.json");
  const fontCss = readFileSync(require("node:path").join(__dirname, "../../node_modules/@xcss/web/dist/web-fonts/fonts.css"), "utf8");
  expect(fontCss).toContain('font-family:"Sarmg Maple"');
  for (const source of Object.keys(provenance.assets).filter(name => (name.endsWith(".woff2") && fontCss.includes(`./${name}`)) || name.endsWith(".txt"))) {
    const name = source.split("/").at(-1);
    // Vite may coalesce identical license bytes into a single emitted asset.
    const url = source === "CJK-LICENSE.txt" ? licenses.cjk
      : ["OFL.txt", "NORMAL-LICENSE.txt"].includes(source) ? licenses.latin
        : new URL(name, prefix).href;
    const response = await page.context().request.get(url);
    expect(response.status()).toBe(200);
    expect(response.headers()["cache-control"]).toContain("immutable");
    expect(response.headers()["x-content-type-options"]).toBe("nosniff");
    expect(response.headers()["content-type"]).toContain(name.endsWith("woff2") ? "font/woff2" : "text/plain");
    expect(await response.body()).toEqual(readFileSync(require("node:path").join(__dirname, "../../node_modules/@xcss/web/dist/web-fonts", source)));
  }
  await page.evaluate(() => document.fonts.ready);
  expect(await page.evaluate(() => document.fonts.check('16px "Sarmg Maple"'))).toBe(true);
  expect(await page.locator("body").evaluate(element => getComputedStyle(element).fontFamily)).toContain("Sarmg Maple");
  expect(await page.evaluate(() => document.documentElement.dataset.xcssFonts)).toBe("ready");
  const faces = await page.evaluate(() => Array.from(document.fonts).map(face => face.status));
  expect(faces.length).toBeGreaterThan(0);
  expect(faces.every(status => status === "loaded")).toBe(true);
  const css = await page.context().request.get(new URL("platform.css", prefix).href);
  const cssText = await css.text();
  expect(cssText).not.toContain("data:image");
  const icons = [...new Set([...cssText.matchAll(/xcss-icon-[a-f0-9]{64}\.svg/gu)].map(match => match[0]))];
  expect(icons).toHaveLength(3);
  for (const icon of icons) {
    const response = await page.context().request.get(new URL(icon, prefix).href);
    expect(response.status()).toBe(200);
    expect(response.headers()["content-type"]).toContain("image/svg+xml");
    expect(response.headers()["cache-control"]).toContain("immutable");
  }
  const script = await page.context().request.get(new URL("platform.js", prefix).href);
  expect((await script.body()).byteLength).toBeLessThanOrEqual(512 * 1024);
  expect(await script.text()).not.toMatch(/sourceMappingURL/u);
  await expect(page.locator('#xczs-root[data-xczs-renderer="leptos"] > header')).toHaveCount(1);
  expect(await page.locator("#xczs-root").evaluate(root => Object.keys(root).some(key => key.startsWith("__reactContainer$")))).toBe(false);
  await expect(page.locator("html")).toHaveAttribute("data-xcss-runtime", "leptos");
  expect(await page.evaluate(() => performance.getEntriesByType("resource").some(entry => new URL(entry.name).pathname.endsWith(".wasm")))).toBe(true);
  const metadataResponse = await page.context().request.get(new URL("../assets/leptos-build.json", prefix).href);
  expect(metadataResponse.status()).toBe(200);
  const metadata = await metadataResponse.json();
  expect(metadata.package).toBe("xczs_web");
  expect(metadata.profile).toBe("release");
  expect(Object.keys(metadata.browser_inputs)).toEqual(expect.arrayContaining([
    "Cargo.toml", "Cargo.lock", "src/lib.rs", "src/http.rs", "src/model.rs",
    "src/operations.rs", "src/protocol.rs", "src/tags.rs", "src/upload.rs", "src/workspace.rs",
  ]));
  expect(Object.keys(metadata.shared_inputs)).toEqual(expect.arrayContaining([
    "Cargo.toml", "SOURCE.json", "src/lib.rs", "src/api.rs", "src/theme.rs",
  ]));
  const sha256 = bytes => createHash("sha256").update(bytes).digest("hex");
  for (const [name, digest] of Object.entries(metadata.browser_inputs)) {
    expect(sha256(readFileSync(require("node:path").join(__dirname, "../../web/leptos", name))), name).toBe(digest);
  }
  for (const [name, digest] of Object.entries(metadata.shared_inputs)) {
    expect(sha256(readFileSync(require("node:path").join(__dirname, "../../web/leptos/vendor/xcss-web", name))), name).toBe(digest);
  }
  expect(sha256(readFileSync(require("node:path").join(__dirname, "../../scripts/build-leptos.py")))).toBe(metadata.builder_sha256);
  const wasmUrl = await page.evaluate(() => performance.getEntriesByType("resource").find(entry => new URL(entry.name).pathname.endsWith(".wasm")).name);
  const wasmResponse = await page.context().request.get(wasmUrl);
  expect(wasmResponse.status()).toBe(200);
  expect(wasmResponse.headers()["content-type"]).toContain("application/wasm");
  expect(sha256(await wasmResponse.body())).toBe(metadata.files["xczs_web_bg.wasm"]);
});

test("Leptos frontend mobile light and dark themes pass WCAG AA", async ({ axePage: page }) => {
  await login(page);
  await page.setViewportSize({ width: 390, height: 844 });
  for (const colorScheme of ["light", "dark"]) {
    await page.emulateMedia({ colorScheme, reducedMotion: "reduce" });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"]).analyze();
    expect(results.violations).toEqual([]);
  }
});

test("The xcss appearance unifies the top product name, equal-height icons, and full-width file content", async ({ appPage: page }, testInfo) => {
  await expect(page.locator(".xcss-page-header .xcss-product-identity")).toHaveText("xczs");
  await expect(page.locator(".xcss-header-navigation a[aria-current=page]")).toHaveText("Files");
  await expect(page.locator(".xcss-instance-sidebar")).toHaveCount(0);
  await page.evaluate(() => document.fonts.ready);
  for (const width of [1280, 768, 320]) {
    await page.setViewportSize({ width, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    const main = await page.locator(".main").boundingBox();
    expect(main.x).toBeLessThanOrEqual(32);
    expect(main.width).toBeGreaterThan(width - 65);
    const responsiveHeader = await page.locator(".xcss-page-header").boundingBox();
    const responsiveSecondary = await page.locator(".main > .xczs-file-actions").boundingBox();
    const responsiveFirstContent = await page.locator(".file-toolbar").boundingBox();
    const responsiveFilePanel = await page.locator(".index-page").boundingBox();
    expect(Math.abs(responsiveSecondary.y - (responsiveHeader.y + responsiveHeader.height))).toBeLessThan(2);
    expect(Math.abs(responsiveFirstContent.y - (responsiveSecondary.y + responsiveSecondary.height) - 16)).toBeLessThan(2);
    expect(Math.abs(responsiveFirstContent.x - main.x - 16)).toBeLessThan(2);
    expect(Math.abs(responsiveFilePanel.x - main.x - 16)).toBeLessThan(2);
    expect(Math.abs(responsiveFilePanel.width - (main.width - 32))).toBeLessThan(2);
  }
  await page.setViewportSize({ width: 1838, height: 900 });
  await expect.poll(() => page.locator(".xcss-header-actions").evaluate(element => element.getBoundingClientRect().height)).toBeLessThan(45);
  const menuSpacing = await page.evaluate(() => {
    const header = document.querySelector(".xcss-page-header").getBoundingClientRect();
    const menu = document.querySelector(".xcss-header-navigation a[aria-current='page']");
    const secondaryControl = document.querySelector(".xczs-root-navigation a").getBoundingClientRect();
    const range = document.createRange();
    range.selectNodeContents(menu);
    const label = range.getBoundingClientRect();
    return { topToMenu: label.top - header.top, menuToSecondary: secondaryControl.top - label.bottom };
  });
  expect(Math.abs(menuSpacing.topToMenu - 16)).toBeLessThan(2);
  expect(Math.abs(menuSpacing.menuToSecondary - 16)).toBeLessThan(2);
  await page.setViewportSize({ width: 1280, height: 900 });
  expect(await page.locator(".xcss-header-navigation a").evaluateAll(nodes =>
    new Set(nodes.map(node => Math.round(node.getBoundingClientRect().top))).size)).toBe(1);
  const menuItems = page.locator(".xczs-file-actions > :is(.xczs-root-navigation, .upload-file, .upload-folder, .new-folder, .new-file, .file-action-mode, .searchbar)");
  await expect(menuItems).toHaveCount(11);
  const menuBar = page.locator(".xcss-page-header .xcss-header-actions");
  await expect(page.locator(".main > .xczs-file-actions.xcss-secondary-navigation")).toHaveCount(1);
  await expect(page.locator(".xcss-page-header .xczs-file-actions")).toHaveCount(0);
  expect(await page.locator(".file-toolbar .xczs-file-actions")).toHaveCount(0);
  await expect.poll(() => menuBar.evaluate(element => {
    const header = element.closest(".xcss-page-header");
    return header !== null
      && element.getBoundingClientRect().bottom <= header.getBoundingClientRect().bottom;
  })).toBe(true);
  const button = page.getByRole("button", { name: "Switch to dark mode", exact: true });
  await button.click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.getByRole("button", { name: "Switch to light mode", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  for (const icon of await page.locator(".xcss-header-actions button > svg").all()) {
    expect(await icon.evaluate(svg => Math.abs(svg.getBoundingClientRect().height - Number.parseFloat(getComputedStyle(svg.parentElement).fontSize)))).toBeLessThan(1);
  }
  expect(await page.locator(".paths-table").evaluate(table => getComputedStyle(table).borderCollapse)).toBe("collapse");
  await page.screenshot({ path: testInfo.outputPath("react-xcss-desktop.png") });
});

test("Leptos theme updates neither rebuild the upload queue nor resend uploads", async ({ appPage: page }) => {
  let releaseUpload;
  let reachedUpload;
  const gate = new Promise(resolve => { releaseUpload = resolve; });
  const reached = new Promise(resolve => { reachedUpload = resolve; });
  let requests = 0;
  await page.route("**/react-theme-upload.txt", async route => {
    if (route.request().method() === "PUT") {
      requests++;
      reachedUpload();
      await gate;
    }
    await route.continue();
  });
  try {
    await selectFiles(page, "#file", [{ name: "react-theme-upload.txt", mimeType: "text/plain", buffer: Buffer.from("React keeps the upload alive") }]);
    await reached;
    const row = await page.locator(".upload-status").elementHandle();
    expect(row).not.toBeNull();
    await page.getByRole("button", { name: "Switch to dark mode", exact: true }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await page.getByRole("button", { name: "Switch to light mode", exact: true }).click();
    expect(await row.evaluate(element => element.isConnected && element === document.querySelector(".upload-status"))).toBe(true);
    expect(requests).toBe(1);
    await page.getByRole("button", { name: "Account settings", exact: true }).click();
    await expect(page.getByRole("region", { name: "Account settings", exact: true })).toBeVisible();
    expect(await row.evaluate(element => element.isConnected)).toBe(true);
    await page.getByRole("navigation", { name: "Main navigation" }).getByRole("link", { name: "Files", exact: true }).click();
    await expect(page.getByRole("region", { name: "Account settings", exact: true })).toHaveCount(0);
    expect(await row.evaluate(element => element.isConnected && element === document.querySelector(".upload-status"))).toBe(true);
    expect(requests).toBe(1);
  } finally { releaseUpload(); }
  await expect(page.locator(".upload-status")).toHaveAttribute("aria-label", "react-theme-upload.txt: upload complete");
  expect(requests).toBe(1);
});
