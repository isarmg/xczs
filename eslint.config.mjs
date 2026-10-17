import js from "@eslint/js";
import nounsanitized from "eslint-plugin-no-unsanitized";
import globals from "globals";

const authoredFiles = [
  "eslint.config.mjs",
  "playwright.config.js",
  "scripts/**/*.mjs",
  "tests/frontend/**/*.{js,mjs}",
  "web/**/*.{js,jsx,mjs}",
];

export default [
  {
    ignores: [
      "node_modules/**",
      "playwright-report/**",
      "target/**",
      "test-results/**",
      "web/dist/**",
      "web/runtime-dist/**",
      "web/pkg/**",
      "web/xcss-web/vendor/xcss-web/vendor/xcss-web-core/**",
    ],
  },
  {
    ...js.configs.recommended,
    files: authoredFiles,
    languageOptions: {
      ecmaVersion: "latest",
      parserOptions: { ecmaFeatures: { jsx: true } },
      globals: {
        ...globals.browser,
        ...globals.node,
      },
      sourceType: "module",
    },
    linterOptions: {
      reportUnusedDisableDirectives: "error",
    },
    rules: {
      ...js.configs.recommended.rules,
      "eol-last": ["error", "always"],
      "no-alert": "error",
      "no-eval": "error",
      "no-implied-eval": "error",
      "no-new-func": "error",
      "no-script-url": "error",
      "no-tabs": "error",
      "no-trailing-spaces": "error",
      "unicode-bom": ["error", "never"],
    },
  },
  {
    files: ["web/**/*.{js,jsx}"],
    ignores: ["web/dist/**", "web/runtime-dist/**"],
    plugins: { nounsanitized },
    rules: {
      "no-restricted-globals": [
        "error",
        { name: "fetch", message: "Use the Rust browser transport." },
        { name: "XMLHttpRequest", message: "Use the Rust upload transport." },
      ],
      "no-restricted-syntax": [
        "error",
        {
          selector: "MemberExpression[property.name='fetch']",
          message: "Use the Rust browser transport.",
        },
        {
          selector: "MemberExpression[property.name='XMLHttpRequest']",
          message: "Use the Rust upload transport.",
        },
        {
          selector: "Property[key.name='dangerouslySetInnerHTML']",
          message: "HTML injection is not allowed.",
        },
        {
          selector: "NewExpression[callee.name='DOMParser']",
          message: "DOMParser is not allowed in authored browser code.",
        },
        {
          selector: "CallExpression[callee.property.name='createContextualFragment']",
          message: "HTML fragment parsing is not allowed.",
        },
        {
          selector: "MemberExpression[property.name='setHTMLUnsafe']",
          message: "Unsafe HTML parsing is not allowed.",
        },
      ],
      "nounsanitized/method": "error",
      "nounsanitized/property": "error",
    },
  },
];
