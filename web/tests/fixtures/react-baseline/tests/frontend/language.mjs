import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { chromium, firefox, expect } from '@playwright/test';

const session = { authenticated: true, user_id: 'A'.repeat(43), username: 'admin', role: 'admin', csrf_token: 'A'.repeat(43) };
const tag = { id: 1, name: '中文标签', color: '#3b82f6', file_count: 1 };
const tagPage = { previous_cursor: null, next_cursor: null, tags: [tag] };
const file = { id: 1, name: '中文原文件.txt', path: '中文目录/中文原文件.txt', status: 'present', size: 2048, mtime_ns: 1_791_446_400_000_000_000, tag_ids: [1], tag_ids_has_more: false };
const authored = [file.path, file.name, tag.name, '中文目录', '/文件根目录'];
const base = 'http://xczs-language.test';
const root = resolve(import.meta.dirname, '../..');
const html = (await readFile(resolve(root, 'web/index.html'), 'utf8'))
  .replaceAll('__ASSETS_PREFIX__', '/__xczs__/')
  .replace('__INDEX_DATA__', Buffer.from(JSON.stringify({ href: '/', dir_exists: true })).toString('base64'));

async function fixture(page) {
  await page.route(base + '/**', async route => {
    const path = new URL(route.request().url()).pathname;
    if (path.startsWith('/api/v1/')) {
      if (route.request().method() !== 'GET') return route.fulfill({ status: 409, json: { code: 'capacity_exhausted', message: '内部 SECRET', retryable: false, request_id: 'capacity-409' } });
      const payload = path === '/api/v1/auth/session' ? session
        : path.endsWith('/tags') ? tagPage
        : path.endsWith('/file') ? { file_id: 1, tags: [tag], tags_has_more: false }
        : path.endsWith('/files') ? { files: [file], total: 1, page: 1, page_size: 50 }
        : path.endsWith('/folders') ? { previous_cursor: null, next_cursor: null, folders: ['中文目录'] }
        : { root: '/文件根目录', available: true, last_scan_at: 1791446400, indexed: 1, missing: 0, suspect: 0, last_error: '内部 SECRET' };
      return route.fulfill({ json: payload });
    }
    if (path === '/__xczs__/api/list') return route.fulfill({ json: { paths: [{ path_type: 'File', name: file.name, mtime: 1791446400000, size: 2048, revision: 'a'.repeat(64) }], next_cursor: null, file_tags: [{ file_id: 1, tags: [tag], tags_has_more: false }] } });
    if (path.startsWith('/__xczs__/')) {
      const name = path.slice('/__xczs__/'.length);
      const type = name.endsWith('.js') ? 'text/javascript' : name.endsWith('.css') ? 'text/css' : name.endsWith('.woff2') ? 'font/woff2' : 'image/svg+xml';
      return route.fulfill({ contentType: type, body: await readFile(resolve(root, 'web/runtime-dist', name)) });
    }
    return route.fulfill({ contentType: 'text/html', body: html });
  });
}
async function english(page) {
  const strip = value => authored.reduce((text, item) => text.replaceAll(item, ''), value);
  assert.doesNotMatch(strip(await page.locator('body').innerText()), /\p{Script=Han}/u);
  const attributes = await page.locator('[aria-label],[title],[placeholder]').evaluateAll(nodes => nodes.flatMap(node => ['aria-label','title','placeholder'].map(name => node.getAttribute(name) || '')).join('\n'));
  assert.doesNotMatch(strip(attributes), /\p{Script=Han}/u);
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  assert.doesNotMatch(await page.locator('body').innerText(), /SECRET/);
}

for (const engine of [chromium, firefox]) {
  const browser = await engine.launch();
  try {
    for (const width of [360, 1280]) {
      const context = await browser.newContext({ locale: 'zh-CN', viewport: { width, height: 900 } });
      const page = await context.newPage(), errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await fixture(page);
      await page.goto(base + '/?lang=zh-CN#files');
      await expect(page.getByRole('table', { name: '文件列表', exact: true })).toBeVisible();
      await page.getByRole('button', { name: '切换为英文', exact: true }).click();
      await expect(page.getByRole('table', { name: 'File list', exact: true })).toBeVisible();
      await expect(page.getByRole('row').filter({ hasText: file.name })).toBeVisible();
      await expect(page).toHaveTitle('/ - xczs File Manager');
      await english(page);
      await page.locator('[data-file-action=tags]').click();
      await page.getByRole('link', { name: file.name, exact: true }).click();
      await expect(page.getByRole('region', { name: 'File tags', exact: true })).toBeVisible();
      await expect(page.locator('.file-tag-target')).toHaveText(file.name);
      await english(page);
      await page.locator('[data-file-action=tags]').click();
      await page.getByRole('link', { name: 'Manage tags', exact: true }).click();
      await expect(page.getByRole('table', { name: 'Tag list', exact: true })).toContainText(tag.name);
      await page.getByRole('button', { name: 'Edit', exact: true }).click();
      await expect(page.getByRole('dialog', { name: 'Edit tag', exact: true })).toBeVisible();
      await english(page);
      await page.getByRole('button', { name: 'Cancel', exact: true }).click();
      await page.getByRole('button', { name: 'Delete tag', exact: true }).click();
      await expect(page.getByRole('dialog')).toContainText('Original files are retained.');
      await page.getByRole('button', { name: 'Confirm', exact: true }).click();
      await expect(page.getByRole('alert')).toContainText('Storage capacity reached.');
      await english(page);
      await page.getByRole('button', { name: 'Cancel', exact: true }).click();
      await page.getByRole('link', { name: 'Service status', exact: true }).click();
      await expect(page.getByText('The scan could not complete. Scan again.', { exact: true })).toBeVisible();
      await english(page);
      await page.getByRole('button', { name: 'Switch to Chinese', exact: true }).click();
      await expect(page.getByText('原文件根目录', { exact: true })).toBeVisible();
      assert.deepEqual(errors, []);
      await context.close();
    }
    console.log(`${engine.name()}: bilingual tags, dialogs, capacity errors, authored names and mobile layout passed`);
  } finally { await browser.close(); }
}
