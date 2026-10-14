import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { parseDocument } from "yaml";

const document = parseDocument(readFileSync(new URL("../.github/workflows/read-only-ci.yml", import.meta.url), "utf8"));
assert.deepEqual(document.errors, []);
const { jobs } = document.toJS();
const gate = jobs.quality.steps[0];

test("quality work runs in parallel with the same scope and required check name", () => {
  assert.equal(jobs.quality.name, "Coverage, deployment, and release smoke");
  assert.deepEqual(jobs.quality.needs, ["classify", "coverage", "release_smoke"]);
  assert.equal(jobs.quality.if, "${{ always() }}");
  for (const id of ["coverage", "release_smoke"]) {
    assert.equal(jobs[id].needs, "classify");
    assert.equal(jobs[id].if, "${{ !cancelled() && (github.event_name != 'pull_request' || needs.classify.outputs.release_sensitive == 'true') }}");
    assert.equal(jobs[id].steps.some(step => step["continue-on-error"]), false);
  }
  const coverage = jobs.coverage.steps.map(step => step.run ?? "").join("\n");
  const release = jobs.release_smoke.steps.map(step => step.run ?? "").join("\n");
  assert.match(coverage, /\.\/scripts\/check-coverage\.sh/u);
  assert.doesNotMatch(coverage, /check-deployment|check-release-runtime/u);
  assert.match(release, /npm ci --ignore-scripts --no-audit --no-fund/u);
  assert.match(release, /npm run build:platform/u);
  assert.match(release, /\.\/scripts\/check-deployment\.sh/u);
  assert.match(release, /npm run build:server:release -- --no-install --rust-only/u);
  assert.match(release, /bash scripts\/check-release-runtime\.sh/u);
  assert.equal(gate.env.COVERAGE_RESULT, "${{ needs.coverage.result }}");
  assert.equal(gate.env.RELEASE_RESULT, "${{ needs.release_smoke.result }}");
  assert.equal(gate.env.CLASSIFY_RESULT, "${{ needs.classify.result }}");
  assert.equal(gate.env.RELEASE_SENSITIVE, "${{ needs.classify.outputs.release_sensitive }}");
});

test("the actual aggregate shell fails closed for every result combination", () => {
  const results = ["success", "failure", "cancelled", "skipped"];
  let checked = 0;
  for (const event of ["push", "workflow_dispatch", "pull_request"]) {
    for (const classify of results) {
      for (const sensitive of ["true", "false", ""]) {
        for (const coverage of results) {
          for (const release of results) {
            const expected = event === "pull_request"
              ? classify === "success" && (
                sensitive === "true" && coverage === "success" && release === "success" ||
                sensitive === "false" && coverage === "skipped" && release === "skipped")
              : coverage === "success" && release === "success";
            const result = spawnSync("bash", ["-c", gate.run], {
              encoding: "utf8",
              env: {
                ...process.env,
                EVENT_NAME: event,
                CLASSIFY_RESULT: classify,
                RELEASE_SENSITIVE: sensitive,
                COVERAGE_RESULT: coverage,
                RELEASE_RESULT: release,
              },
            });
            assert.ifError(result.error);
            assert.equal(result.status === 0, expected,
              JSON.stringify({ event, classify, sensitive, coverage, release }));
            checked++;
          }
        }
      }
    }
  }
  assert.equal(checked, 576);
});

test("isolated browser shards preserve the old required check names and failure propagation", () => {
  const matrix = jobs.browser_tests.strategy;
  assert.equal(matrix["fail-fast"], false);
  assert.deepEqual(matrix.matrix.include, [
    { browser: "chromium", shard: 1, shards: 1 },
    { browser: "firefox", shard: 1, shards: 2 },
    { browser: "firefox", shard: 2, shards: 2 },
  ]);
  assert.equal(jobs.browser_tests.steps.some(step => step["continue-on-error"]), false);
  const run = jobs.browser_tests.steps.find(step => step.name === "Build the candidate and run the isolated browser project");
  assert.equal(run.run, "node scripts/build-frontend-candidate.mjs --project=\"$BROWSER\" --shard=\"$SHARD/$SHARDS\"");
  assert.equal(run.env.SHARD, "${{ matrix.shard }}");
  assert.equal(run.env.SHARDS, "${{ matrix.shards }}");
  const diagnostics = jobs.browser_tests.steps.find(step => step.name === "Preserve browser diagnostics on failure");
  assert.ok(diagnostics.with.name.includes("${{ matrix.shard }}"));
  assert.equal(jobs.browser.name, "Browser (${{ matrix.browser }})");
  assert.equal(jobs.browser.strategy["fail-fast"], false);
  assert.deepEqual(jobs.browser.strategy.matrix.browser, ["chromium", "firefox"]);
  assert.equal(jobs.browser.needs, "browser_tests");
  assert.equal(jobs.browser.if, "${{ always() }}");
  const aggregate = jobs.browser.steps[0];
  assert.equal(aggregate.env.BROWSER_RESULT, "${{ needs.browser_tests.result }}");
  for (const status of ["success", "failure", "cancelled", "skipped"]) {
    const result = spawnSync("bash", ["-c", aggregate.run], {
      env: { ...process.env, BROWSER_RESULT: status },
      encoding: "utf8",
    });
    assert.ifError(result.error);
    assert.equal(result.status === 0, status === "success");
  }
});

function listBrowserTests(browser, shard) {
  const args = ["tests/frontend/run.mjs", `--project=${browser}`, "--list", "--reporter=json"];
  if (shard) args.push(`--shard=${shard}`);
  const result = spawnSync(process.execPath, args, {
    cwd: new URL("..", import.meta.url),
    encoding: "utf8",
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  const report = JSON.parse(result.stdout);
  assert.deepEqual(report.errors, []);
  assert.equal(report.config.workers, 1, "shared login buckets require one worker per server");
  assert.equal(report.config.fullyParallel, false, "preserve within-file ordering");
  const ids = [];
  function visit(suites) {
    for (const suite of suites) {
      for (const spec of suite.specs ?? []) {
        for (const testCase of spec.tests) {
          assert.equal(testCase.projectName, browser);
          ids.push(`${spec.id}:${testCase.projectName}`);
        }
      }
      visit(suite.suites ?? []);
    }
  }
  visit(report.suites);
  assert.ok(ids.length > 0, "each browser shard must contain real tests");
  return ids.sort();
}

test("real Playwright discovery proves shard completeness, uniqueness, and serial isolation", () => {
  for (const browser of ["chromium", "firefox"]) {
    const full = listBrowserTests(browser);
    const shards = jobs.browser_tests.strategy.matrix.include.filter(entry => entry.browser === browser);
    const parts = shards.map(entry => listBrowserTests(browser, `${entry.shard}/${entry.shards}`));
    const combined = parts.flat().sort();
    assert.equal(new Set(combined).size, combined.length, "shards must not duplicate tests");
    assert.deepEqual(combined, full, "shards must cover the entire original project");
    // File-level sharding preserves ordering; prevent an accidentally empty or
    // grossly unbalanced shard from silently losing the intended speedup.
    if (parts.length > 1) {
      for (const part of parts) assert.ok(part.length >= full.length / 3);
    }
    process.stdout.write(`${browser}: ${full.length} tests; shards ${parts.map(part => part.length).join(" + ")}\n`);
  }
});


test("push release smoke reuses its candidate for real installation with exact cached tools", () => {
  const workflow = document.toJS();
  assert.deepEqual(workflow.on.push.branches, ["main"]);
  const steps = jobs.release_smoke.steps;
  const commands = steps.map(step => step.run ?? "").join("\n");
  assert.equal((commands.match(/npm run build:server:release/g) ?? []).length, 1);
  assert.doesNotMatch(commands, /check-formal-release-e2e|scripts\/package-release\.sh|gh release|git tag/u);
  assert.match(commands, /XSSI_BUILD_METADATA_DIR=.*npm run build:server:release/u);
  const candidate = steps.findIndex(step => step.run?.includes("bash scripts/package-installed-candidate.sh"));
  const runtime = steps.findIndex(step => step.run?.includes("bash scripts/check-release-runtime.sh"));
  assert.ok(candidate > runtime && runtime >= 0);
  assert.match(steps[candidate].run, /sudo env GITHUB_ACTIONS=.*RUNNER_ENVIRONMENT=/u);
  assert.match(steps[candidate].run, /check-installed-lifecycle\.py/u);
  assert.match(steps[candidate].run, /LIFECYCLE_DIAGNOSTICS=/u);
  const cache = steps.find(step => step.id === "inventory_tools");
  assert.match(cache.with.key, /runner\.os.*runner\.arch.*rust-1\.99\.0-audit-0\.22\.2-cyclonedx-0\.5\.9/u);
  assert.equal(cache.with["restore-keys"], undefined);
  assert.equal(cache.uses, "actions/cache/restore@0400d5f644dc74513175e3cd8d07132dd4860809");
  const saved = steps.find(step => step.name === "Save verified inventory tools before testing the candidate");
  assert.equal(saved.uses, "actions/cache/save@0400d5f644dc74513175e3cd8d07132dd4860809");
  assert.equal(saved.if, "${{ steps.inventory_tools.outputs.cache-hit != 'true' && steps.install_tools.outcome == 'success' }}");
  assert.equal(saved.with.path, cache.with.path);
  assert.equal(saved.with.key, "${{ steps.inventory_tools.outputs.cache-primary-key }}");
  assert.ok(steps.indexOf(saved) > steps.findIndex(step => step.id === "install_tools"));
  assert.ok(steps.indexOf(saved) < candidate);
  const evidence = steps.find(step => step.name === "Retain redacted installation lifecycle evidence");
  assert.equal(evidence.if, "${{ always() }}");
  assert.equal(evidence.with.path, "${{ runner.temp }}/install-diagnostics");
});


test("release Rust cache reuses only exact dependencies and always rebuilds the candidate", () => {
  const steps = jobs.release_smoke.steps;
  const cache = steps.find(step => step.id === "release_rust_cache");
  const environment = steps.find(step => step.id === "release_rust_environment");
  const clean = steps.find(step => step.name === "Discard cached candidate products before real compilation");
  const runtime = steps.find(step => step.id === "release_binary");
  const save = steps.find(step => step.name === "Save compiled Rust dependencies before installation tests");
  const candidate = steps.find(step => step.run?.includes("bash scripts/package-installed-candidate.sh"));
  assert.equal(jobs.release_smoke.env.CARGO_INCREMENTAL, "0");
  assert.equal(cache.uses, "actions/cache/restore@0400d5f644dc74513175e3cd8d07132dd4860809");
  assert.equal(save.uses, "actions/cache/save@0400d5f644dc74513175e3cd8d07132dd4860809");
  assert.deepEqual(cache.with.path.trim().split("\n"), [
    "~/.cargo/registry/cache", "~/.cargo/registry/index", "~/.cargo/git/db", "target",
  ]);
  assert.equal(cache.with["restore-keys"], undefined);
  for (const input of ["runner.os", "runner.arch", "env.RUST_TOOLCHAIN", "env.SERVER_TARGET",
    "debug-release", "steps.release_rust_environment.outputs.fingerprint", "Cargo.lock",
    "**/Cargo.toml", "**/build.rs", ".cargo/**", "config/xssi.lock", "package-lock.json",
    "xcss-web-build.json", ".github/workflows/read-only-ci.yml"]) {
    assert.ok(cache.with.key.includes(input), `missing cache identity: ${input}`);
  }
  for (const input of ["ImageOS", "ImageVersion", "rustc --version --verbose",
    "cargo --version --verbose", "CARGO_", "RUST", "CFLAGS", "LDFLAGS"]) {
    assert.ok(environment.run.includes(input), `missing environment identity: ${input}`);
  }
  assert.equal(environment.env.RUSTUP_TOOLCHAIN, "${{ env.RUST_TOOLCHAIN }}");
  assert.equal(clean.env.RUSTUP_TOOLCHAIN, "${{ env.RUST_TOOLCHAIN }}");
  assert.match(clean.run, /cargo clean --package xczs\n/u);
  assert.match(clean.run, /cargo clean --package xczs --release --target "\$SERVER_TARGET"/u);
  assert.equal(clean.if, undefined, "candidate cleanup cannot depend on cache hit");
  assert.doesNotMatch(runtime.if, /cache/u, "a cache hit must never bypass the real build");
  assert.match(runtime.run, /npm run build:server:release -- --no-install --rust-only/u);
  assert.match(runtime.run, /bash scripts\/check-release-runtime\.sh/u);
  assert.equal(save.if, "${{ github.event_name == 'push' && github.ref == 'refs/heads/main' && steps.release_rust_cache.outputs.cache-hit != 'true' && steps.release_binary.outcome == 'success' }}");
  assert.equal(save.with.key, "${{ steps.release_rust_cache.outputs.cache-primary-key }}");
  assert.equal(save.with.path, cache.with.path);
  assert.ok(steps.indexOf(environment) < steps.indexOf(cache));
  assert.ok(steps.indexOf(cache) < steps.indexOf(clean));
  assert.ok(steps.indexOf(clean) < steps.findIndex(step => step.name === "Run deployment checks"));
  assert.ok(steps.indexOf(clean) < steps.indexOf(runtime));
  assert.ok(steps.indexOf(runtime) < steps.indexOf(save));
  assert.ok(steps.indexOf(save) < steps.indexOf(candidate));
});
