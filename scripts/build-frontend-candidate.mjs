import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
const printPath = process.argv[2] === "--print-path";
if (printPath && process.argv.length !== 3) throw new Error("--print-path does not accept browser runner arguments");
const root = fileURLToPath(new URL("../", import.meta.url));
const result = spawnSync("cargo", ["+1.99.0", "run", "--quiet", "--locked", "--manifest-path", "tools/web-build/Cargo.toml", "--", "server", "--product-root", root, "--config", "xcss-web-build.json", "--mode", "development"], {cwd: root, env: process.env, encoding: "utf8", stdio: ["inherit", "pipe", "inherit"]});
if (result.error) throw result.error;
if (result.status !== 0) throw new Error(`Rust candidate build failed (${result.status ?? result.signal})`);
const binary = result.stdout.trim().split("\n").at(-1);
if (!binary?.startsWith("/")) throw new Error("Rust builder returned no absolute executable path");
if (printPath) process.stdout.write(`${binary}\n`);
else { process.env.XCZS_FRONTEND_BINARY = binary; await import("./run-frontend-prepared.mjs"); }
