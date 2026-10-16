import { spawnSync } from "node:child_process";
import "./check-xcss-web.mjs";
import { cpSync, lstatSync, mkdtempSync, rmSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { assertWebOutput } from "@xcss/web/web-toolchain/server";

// Vite interprets '#' in physical filenames as a URL fragment. Build the
// exact installed inputs in a fresh private, URL-safe scratch directory. The
// release output and its held-directory publication protocol stay unchanged.
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const output = join(root, "web/dist");
const defaultRuntimeOutput = join(root, "web/runtime-dist");
const runtimeOutput = resolve(process.env.XCSS_WEB_DIST ?? defaultRuntimeOutput);
assertWebOutput(root, root, defaultRuntimeOutput, runtimeOutput);
const scratch = mkdtempSync("/tmp/xczs-platform-build-");

try {
  const build = spawnSync("python3", ["scripts/build-leptos.py", "build", ...(process.env.XCZS_LEPTOS_DEBUG === "1" ? ["--debug"] : []), ...(process.env.CARGO_NET_OFFLINE === "true" ? ["--offline"] : [])], { cwd: root, env: process.env, stdio: "inherit" });
  if (build.error) throw build.error;
  if (build.status !== 0) throw new Error("Leptos/WASM compilation failed");
  for (const directory of ["web", "node_modules"]) {
    const metadata = lstatSync(join(root, directory));
    if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
      throw new Error(`Build input is not a real directory: ${directory}`);
    }
  }
  cpSync(join(root, "web"), join(scratch, "web"), {
    recursive: true,
    filter: source => !["dist", "runtime-dist", "types", "tests"].includes(relative(join(root, "web"), source).split("/")[0]),
  });
  cpSync(join(root, "node_modules"), join(scratch, "node_modules"), { recursive: true });
  for (const file of ["package.json", "vite.platform.config.mjs"]) {
    cpSync(join(root, file), join(scratch, file));
  }
  const result = spawnSync(process.execPath, [
    join(scratch, "node_modules/vite/bin/vite.js"),
    "build", "--config", "vite.platform.config.mjs",
  ], { cwd: scratch, env: { ...process.env, XCSS_WEB_DIST: "" }, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`Platform build failed: ${result.status ?? result.signal}`);

  const previous = lstatSync(output, { throwIfNoEntry: false });
  if (previous && (!previous.isDirectory() || previous.isSymbolicLink())) {
    throw new Error("Refusing to replace a non-directory or linked platform output");
  }
  // dist is generated output only; never remove source or dependency inputs.
  rmSync(output, { recursive: true, force: true });
  cpSync(join(scratch, "web/dist"), output, { recursive: true, errorOnExist: true });
  const existing = lstatSync(runtimeOutput, { throwIfNoEntry: false });
  if (existing && (!existing.isDirectory() || existing.isSymbolicLink())) throw new Error("Unsafe runtime output");
  rmSync(runtimeOutput, { recursive: true, force: true });
  // All executable browser code is compiled by Vite. Static product assets
  // keep their existing URLs for the server-authored HTML templates.
  for (const name of ["index.css", "login.css", "favicon.ico"]) {
    cpSync(join(root, "web", name), join(output, name));
  }
  cpSync(output, runtimeOutput, { recursive: true });
  if (process.env.XSSI_BUILD_TOOLS_DIR) {
    if (!process.env.XSSI_FRONTEND_METADATA_FILE) throw new Error("Explicit runtime inventory output is required");
    const { completeRuntimeInventory } = await import(pathToFileURL(join(process.env.XSSI_BUILD_TOOLS_DIR,"scripts/frontend_build.mjs")));
    completeRuntimeInventory({metadata:process.env.XSSI_FRONTEND_METADATA_FILE,root,distribution:runtimeOutput});
  }
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
