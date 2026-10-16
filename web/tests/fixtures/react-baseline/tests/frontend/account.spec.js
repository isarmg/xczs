const { test, expect } = require('./fixtures');

test('The person icon updates the single administrator while preserving file access', async ({ appPage: page }) => {
  test.slow();
  const originalUrl = page.url();
  const entry = page.getByRole('button', { name: 'Account settings', exact: true });
  await expect(entry).toBeVisible();
  await expect(page.getByRole('button', { name: /Create administrator|创建管理员/ })).toHaveCount(0);
  const appearance = await entry.evaluate(node => ({image:node.querySelector('svg').innerHTML,decoration:getComputedStyle(node).textDecorationLine}));
  await entry.click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page).toHaveURL(/#account$/);
  expect(await entry.evaluate(node => ({image:node.querySelector('svg').innerHTML,decoration:getComputedStyle(node).textDecorationLine}))).toEqual(appearance);
  await expect(page.locator('.xcss-header-navigation [aria-current="page"]')).toHaveCount(0);
  const dialog = page.getByRole('region', { name: 'Account settings', exact: true });
  await expect(dialog.getByLabel('Username', { exact: true })).toHaveValue('frontend-test-0');
  await dialog.getByLabel('Username', { exact: true }).fill('renamed-admin');
  await dialog.getByLabel('Current password', { exact: true }).fill('test-password');
  await dialog.getByLabel('New password', { exact: true }).fill('updated test password');
  await dialog.getByLabel('Confirm new password', { exact: true }).fill('updated test password');
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page).toHaveURL(/\/__xczs__\/login$/);
  await page.getByLabel('Username', { exact: true }).fill('renamed-admin');
  await page.getByLabel('Password', { exact: true }).fill('updated test password');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(page.locator('.index-page')).toBeVisible();
  await page.goto(originalUrl);
  await expect(page.getByRole('link', { name: 'existing-folder', exact: true })).toBeVisible();
  // Return the isolated server's fixture account for the remaining cases.
  await page.getByRole('button', { name: 'Account settings', exact: true }).click();
  await dialog.getByLabel('Username', { exact: true }).fill('frontend-test-0');
  await dialog.getByLabel('Current password', { exact: true }).fill('updated test password');
  await dialog.getByLabel('New password', { exact: true }).fill('test-password');
  await dialog.getByLabel('Confirm new password', { exact: true }).fill('test-password');
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page).toHaveURL(/\/__xczs__\/login$/);
});
