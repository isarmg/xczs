import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { ESLint } from "eslint";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const eslint = new ESLint({ cwd: projectRoot });
const results = await eslint.lintFiles([
  "eslint.config.mjs",
  "playwright.config.js",
  "scripts/**/*.mjs",
  "tests/frontend/**/*.{js,mjs}",
  "web/**/*.{js,mjs,jsx}",
]);
const errors = results.reduce((total, result) => total + result.errorCount, 0);
if (errors > 0) {
  const formatter = await eslint.loadFormatter("stylish");
  process.stderr.write(await formatter.format(results));
  process.exit(1);
}

const packagePath = resolve(projectRoot, "package.json");
const packageSource = readFileSync(packagePath, "utf8");
const normalizedPackage = `${JSON.stringify(JSON.parse(packageSource), null, 2)}\n`;
if (packageSource !== normalizedPackage) {
  throw new Error("package.json must use deterministic two-space JSON formatting");
}

process.stdout.write(
  `ESLint syntax, source formatting, and browser safety checks passed for ` +
    `${results.length} authored files\n`,
);
