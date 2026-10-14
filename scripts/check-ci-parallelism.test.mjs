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
    assert.equal(jobs[id].if, "${{ always() && (github.event_name != 'pull_request' || needs.classify.outputs.release_sensitive == 'true') }}");
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
