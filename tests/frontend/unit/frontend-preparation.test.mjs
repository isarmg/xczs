import assert from "node:assert/strict";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";

function scenario({ prepared = false, buildExit = 0, artifact = true } = {}) {
  const root = mkdtempSync(join(tmpdir(), "xczs-frontend-prepare-"));
  try {
    mkdirSync(join(root, "scripts"));
    mkdirSync(join(root, "tests/frontend"), { recursive: true });
    mkdirSync(join(root, "bin"));
    for (const name of ["run-frontend-prepared.mjs", "prepare-frontend-fixture.mjs"]) {
      copyFileSync(new URL(`../../../scripts/${name}`, import.meta.url), join(root, "scripts", name));
    }
    const candidate = join(root, "candidate");
    const fixture = join(root, "fixture");
    const events = join(root, "events");
    writeFileSync(candidate, '#!/bin/sh\nprintf "candidate-version\\n" >> "$EVENTS"\nprintf "xczs 1.0.1\\n"\n', { mode: 0o700 });
    writeFileSync(fixture, "#!/bin/sh\nexit 0\n", { mode: 0o700 });
    const message = JSON.stringify({ reason: "compiler-artifact", target: { name: "offline_fixture", kind: ["example"] }, executable: fixture });
    writeFileSync(join(root, "bin/cargo"), `#!/bin/sh
printf 'fixture-build %s %s\\n' "$*" "$XCZS_SOURCE_REVISION" >> "$EVENTS"
printf '%s\\n' '${artifact ? message : '{"reason":"build-finished","success":true}'}'
exit ${buildExit}
`, { mode: 0o700 });
    writeFileSync(join(root, "tests/frontend/run.mjs"), `import { appendFileSync } from 'node:fs';
appendFileSync(process.env.EVENTS, 'playwright ' + process.env.XCZS_FRONTEND_BINARY + ' ' + process.env.XCZS_FRONTEND_FIXTURE_BINARY + '\\n');
`);
    const env = { ...process.env, PATH: `${join(root, "bin")}:${process.env.PATH}`, EVENTS: events, XCZS_FRONTEND_BINARY: candidate };
    delete env.XCZS_FRONTEND_FIXTURE_BINARY;
    if (prepared) env.XCZS_FRONTEND_FIXTURE_BINARY = fixture;
    const result = spawnSync(process.execPath, [join(root, "scripts/run-frontend-prepared.mjs")], { cwd: root, env, encoding: "utf8" });
    assert.ifError(result.error);
    return { ...result, events: readFileSync(events, "utf8").trim().split("\n"), candidate, fixture };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test("candidate is reused and fixture compilation finishes before Playwright starts", () => {
  const result = scenario();
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.events, [
    "candidate-version",
    "fixture-build +1.99.0 build --locked --target x86_64-unknown-linux-gnu --example offline_fixture --message-format=json unbound",
    `playwright ${result.candidate} ${result.fixture}`,
  ]);
});

test("an already prepared fixture never invokes Cargo", () => {
  const result = scenario({ prepared: true, buildExit: 99 });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.events, ["candidate-version", `playwright ${result.candidate} ${result.fixture}`]);
});

test("fixture build failure stops before Playwright", () => {
  const result = scenario({ buildExit: 17 });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Frontend fixture build failed \(17\)/u);
  assert.equal(result.events.some(line => line.startsWith("playwright")), false);
});

test("successful Cargo exit without the fixture artifact fails closed", () => {
  const result = scenario({ artifact: false });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /no unique offline_fixture executable/u);
  assert.equal(result.events.some(line => line.startsWith("playwright")), false);
});

test("webServer never compiles inside its unchanged startup deadline", () => {
  const server = readFileSync(new URL("../server.mjs", import.meta.url), "utf8");
  assert.match(server, /if \(!preparedFixture\)/u);
  assert.match(server, /spawnSync\(preparedFixture, fixtureArguments/u);
  assert.doesNotMatch(server, /"cargo"/u);
  const config = readFileSync(new URL("../../../playwright.config.js", import.meta.url), "utf8");
  assert.match(config, /timeout: 120_000/u);
});
