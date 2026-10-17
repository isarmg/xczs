// Compatibility entrypoint for existing developer scripts. Production invokes Cargo directly.
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
const root = fileURLToPath(new URL("../", import.meta.url));
const args = ["+1.99.0", "run", "--quiet", "--locked", "--manifest-path", "tools/web-build/Cargo.toml", "--", "build", "--product-root", root];
if (process.env.XCZS_LEPTOS_DEBUG === "1") args.push("--debug");
if (process.env.CARGO_NET_OFFLINE === "true") args.push("--offline");
const result = spawnSync("cargo", args, {cwd: root, env: process.env, stdio: "inherit"});
if (result.error) throw result.error;
if (result.status !== 0) throw new Error(`Rust browser build failed (${result.status ?? result.signal})`);
