const { test } = require("@playwright/test");

test("resource startup failures stay visible and recover only after an explicit reload", async ({ browser, baseURL }) => {
  test.setTimeout(120_000);
  const { verifyStartupFailures, verifyStartupTimeout } = await import("../../web/xcss-web/vendor/xcss-web/tests/startup-failures.mjs");
  const url = `${baseURL}/__xczs__/login?lang=en`;
  await verifyStartupFailures(browser, url, undefined, { ignoreHTTPSErrors: true });
  await verifyStartupTimeout(browser, url, undefined, { ignoreHTTPSErrors: true });
});
