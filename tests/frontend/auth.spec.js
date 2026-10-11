const { expect, pageData, test } = require("./fixtures");

for (const [locale, usernameLabel, passwordLabel, submitLabel, usernameError, passwordError] of [
  ["en", "Username", "Password", "Sign in", "Enter your username.", "Enter your password."],
  ["zh-CN", "用户名", "密码", "登录", "请输入用户名。", "请输入密码。"],
]) {
  test(`Required login messages appear on the fifth row without native validation bubbles (${locale})`, async ({ browser }, testInfo) => {
    const context = await browser.newContext({ ignoreHTTPSErrors: true });
    try {
      const page = await context.newPage();
      let requests = 0;
      page.on("request", request => {
        if (request.method() === "POST" && new URL(request.url()).pathname === "/api/v1/auth/login") requests++;
      });
      await page.goto(`${testInfo.project.use.baseURL}/__xczs__/login?lang=${locale}`);
      const username = page.getByLabel(usernameLabel, { exact: true });
      const password = page.getByLabel(passwordLabel, { exact: true });
      const submit = page.getByRole("button", { name: submitLabel, exact: true });
      await expect(username).toBeVisible();
      await expect(page.locator(".login-card")).toHaveAttribute("novalidate", "");
      await expect(username).toHaveAttribute("maxlength", "64");
      await expect(username).toHaveAttribute("required", "");
      await expect(password).toHaveAttribute("required", "");
      await expect(password).toHaveAttribute("data-min-bytes", "12");
      await expect(password).toHaveAttribute("data-max-bytes", "1024");
      await page.evaluate(() => {
        window.nativeInvalidCount = 0;
        document.addEventListener("invalid", () => window.nativeInvalidCount++, true);
      });
      await submit.click();
      const error = page.getByRole("alert");
      await expect(error).toHaveText(usernameError);
      await expect(username).toBeFocused();
      await expect(username).toHaveAccessibleDescription(usernameError);
      await username.fill("admin");
      await expect(error).toBeHidden();
      await password.press("Enter");
      await expect(error).toHaveText(passwordError);
      await expect(password).toBeFocused();
      await expect(password).toHaveAttribute("aria-invalid", "true");
      await expect(password).toHaveAccessibleDescription(passwordError);
      const [inputBox, errorBox, buttonBox] = await Promise.all([password.boundingBox(), error.boundingBox(), submit.boundingBox()]);
      expect(errorBox.y).toBeGreaterThanOrEqual(inputBox.y + inputBox.height - 1);
      expect(errorBox.y + errorBox.height).toBeLessThanOrEqual(buttonBox.y + 1);
      expect(await page.evaluate(() => window.nativeInvalidCount)).toBe(0);
      expect(requests).toBe(0);
    } finally { await context.close(); }
  });
}

test("The server enforces login errors, session cookies, and logout", async ({
  browser,
  context,
  appPage: page,
}, testInfo) => {
  // This scenario intentionally performs several Argon2-backed login attempts,
  // session rotation, logout, and replay checks. Firefox can exceed the generic
  // 30-second UI-test budget on constrained builders even when every individual
  // request remains within its protocol deadline.
  test.slow();
  const usernameValue = (await pageData(page)).session.username;
  const anonymous = await browser.newContext({ ignoreHTTPSErrors: true });
  const loginPage = await anonymous.newPage();
  await loginPage.goto(testInfo.project.use.baseURL);
  await expect(loginPage).toHaveURL(/\/__xczs__\/login$/);

  const username = loginPage.getByLabel("Username");
  const password = loginPage.getByLabel("Password");
  const submit = loginPage.getByRole("button", { name: "Sign in" });
  const alert = loginPage.getByRole("alert");

  let emptyLoginPosts = 0;
  loginPage.on("request", request => {
    if (
      request.method() === "POST" &&
      new URL(request.url()).pathname === "/api/v1/auth/login"
    ) {
      emptyLoginPosts++;
    }
  });
  await submit.click();
  await expect(loginPage).toHaveURL(/\/__xczs__\/login$/);
  await expect(username).toBeFocused();
  expect(emptyLoginPosts).toBe(0);
  await expect(alert).toHaveText("Enter your username.");
  await expect(username).toHaveAttribute("aria-invalid", "true");
  await expect(username).toHaveAttribute("aria-describedby", "login-error");

  await username.fill(usernameValue);
  await expect(alert).toBeHidden();
  await password.press("Enter");
  await expect(alert).toHaveText("Enter your password.");
  await expect(password).toBeFocused();
  expect(emptyLoginPosts).toBe(0);
  await password.fill("wrong-password");
  const rejectedLogin = loginPage.waitForResponse(response =>
    response.request().method() === "POST" &&
    new URL(response.url()).pathname === "/api/v1/auth/login"
  );
  await submit.click();
  expect((await rejectedLogin).status()).toBe(401);
  await expect(alert).toContainText("Authentication request could not be completed.");
  await expect(password).toHaveValue("");
  await expect(password).toBeFocused();
  expect(
    (await anonymous.cookies()).find(
      cookie => cookie.name === "__Host-admin-xczs-session",
    ),
  ).toBeUndefined();
  await anonymous.close();

  const sessionCookie = (await context.cookies()).find(
    cookie => cookie.name === "__Host-admin-xczs-session",
  );
  expect(sessionCookie).toMatchObject({
    httpOnly: true,
    secure: true,
    sameSite: "Strict",
    path: "/",
  });

  const logoutResponse = page.waitForResponse(
    response =>
      response.request().method() === "POST" &&
      new URL(response.url()).pathname === "/api/v1/auth/logout",
  );
  await Promise.all([
    page.waitForURL(/\/__xczs__\/login$/),
    page.getByRole("button", { name: "Sign out" }).click(),
  ]);
  expect((await logoutResponse).status()).toBe(204);
  expect(
    (await context.cookies()).find(
      cookie => cookie.name === "__Host-admin-xczs-session",
    ),
  ).toBeUndefined();

  const replay = await browser.newContext({ ignoreHTTPSErrors: true });
  await replay.addCookies([sessionCookie]);
  const replayPage = await replay.newPage();
  await replayPage.goto(testInfo.project.use.baseURL);
  await expect(replayPage).toHaveURL(/\/__xczs__\/login$/);
  await replay.close();
});

test("The login card maintains a 3:2 layout and visible keyboard controls", async ({ page }, testInfo) => {
  await page.goto(`${testInfo.project.use.baseURL}/__xczs__/login`);
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await expect(page).toHaveTitle("Sign in");
  const card = page.locator(".xcss-auth-card");
  const bounds = await card.boundingBox();
  expect(bounds.width / bounds.height).toBeCloseTo(1.5, 2);
  await expect(page.getByLabel("Username")).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(page.getByLabel("Password")).toBeFocused();
  await page.keyboard.press("Tab");
  const submit = page.getByRole("button", { name: "Sign in" });
  await expect(submit).toBeFocused();

  await page.emulateMedia({ forcedColors: "active" });
  expect(
    await page.evaluate(() => matchMedia("(forced-colors: active)").matches),
  ).toBe(true);
  // Corner decoration restores authored borders on its scheduled paint frame.
  await expect.poll(
    () => card.evaluate(element => {
      const style = getComputedStyle(element);
      return style.borderStyle !== "none" &&
        Number.parseFloat(style.borderWidth) >= 1;
    }),
  ).toBe(true);
  const username = page.getByLabel("Username");
  await username.focus();
  for (const control of [username, submit]) {
    expect(
      await control.evaluate(element => {
        const style = getComputedStyle(element);
        return (style.borderBottomStyle !== "none" && Number.parseFloat(style.borderBottomWidth) >= 1) ||
          (style.outlineStyle !== "none" && Number.parseFloat(style.outlineWidth) >= 2);
      }),
    ).toBe(true);
  }
  expect(
    await username.evaluate(element => {
      const style = getComputedStyle(element);
      return style.outlineStyle !== "none" &&
        Number.parseFloat(style.outlineWidth) >= 2;
    }),
  ).toBe(true);
});

test("Browser login password limits use UTF-8 bytes rather than character counts", async ({
  browser,
}, testInfo) => {
  const anonymous = await browser.newContext({ ignoreHTTPSErrors: true });
  const page = await anonymous.newPage();
  const response = await page.goto(
    `${testInfo.project.use.baseURL}/__xczs__/login`,
  );
  expect(response.headers()["content-security-policy"]).toContain(
    "script-src 'self'",
  );
  expect(response.headers()["content-security-policy"]).toContain(
    "connect-src 'self'",
  );

  const password = page.getByLabel("Password");
  await expect(password).toHaveAttribute("data-min-bytes", "12");
  await expect(password).toHaveAttribute("data-max-bytes", "1024");
  expect(await password.getAttribute("maxlength")).toBeNull();

  for (const value of ["p".repeat(1024), "é".repeat(512)]) {
    await password.fill(value);
    expect(
      await password.evaluate(input => ({
        bytes: new TextEncoder().encode(input.value).length,
        message: input.validationMessage,
      })),
    ).toEqual({ bytes: 1024, message: "" });
  }

  let loginPosts = 0;
  page.on("request", request => {
    if (
      request.method() === "POST" &&
      new URL(request.url()).pathname === "/api/v1/auth/login"
    ) {
      loginPosts += 1;
    }
  });
  await page.getByLabel("Username").fill("frontend-test-0");
  await password.fill(`a${"é".repeat(512)}`);
  expect(
    await password.evaluate(input => ({
      bytes: new TextEncoder().encode(input.value).length,
      message: input.validationMessage,
    })),
  ).toEqual({
    bytes: 1025,
    message: "",
  });
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).toHaveURL(/\/__xczs__\/login$/);
  expect(loginPosts).toBe(0);
  await expect(page.getByRole("alert")).toHaveText("Enter a valid administrator username and password.");
  await expect(password).toHaveValue("");
  await expect(password).toBeFocused();

  await anonymous.close();
});
