import { spawnSync } from "node:child_process";
import { accessSync, constants, realpathSync } from "node:fs";

// Compile outside Playwright's webServer deadline. Ask Cargo for the actual
// executable so custom CARGO_TARGET_DIR and Cargo configuration remain valid.
export function prepareFrontendFixture(root, env = process.env) {
  let configured = env.XCZS_FRONTEND_FIXTURE_BINARY;
  if (!configured) {
    const result = spawnSync("cargo", [
      "+1.99.0", "build", "--locked", "--target", "x86_64-unknown-linux-gnu",
      "--example", "offline_fixture", "--message-format=json",
    ], {
      cwd: root,
      env: { ...env, XCZS_SOURCE_REVISION: "unbound" },
      encoding: "utf8",
      maxBuffer: 16 * 1024 * 1024,
      stdio: ["ignore", "pipe", "inherit"],
    });
    if (result.error) throw result.error;
    if (result.status !== 0) {
      throw new Error(`Frontend fixture build failed (${result.status ?? result.signal})`);
    }
    const artifacts = result.stdout.trim().split("\n").filter(Boolean).map(line => JSON.parse(line));
    const executables = artifacts.filter(artifact => artifact.reason === "compiler-artifact"
      && artifact.target?.name === "offline_fixture"
      && artifact.target.kind?.includes("example")
      && typeof artifact.executable === "string");
    if (executables.length !== 1) throw new Error("Cargo returned no unique offline_fixture executable");
    configured = executables[0].executable;
  }
  const binary = realpathSync(configured);
  accessSync(binary, constants.X_OK);
  return binary;
}
