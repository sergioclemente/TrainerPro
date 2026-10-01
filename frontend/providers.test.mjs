import assert from "node:assert/strict";
import { build } from "esbuild";
import { before, test } from "node:test";

let refreshPlanConnections;
before(async () => {
  const result = await build({
    entryPoints: [new URL("./providers.ts", import.meta.url).pathname],
    bundle: true, format: "esm", platform: "node", write: false,
  });
  ({ refreshPlanConnections } = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`));
});

function connection(id, capabilities, state = "connected") {
  return { connection_id: id, provider: { id, name: id, capabilities }, state, error: null };
}

test("a failed source does not prevent another source or a dual-capability connection refreshing", async () => {
  const calls = [];
  const report = { inserted: 1, issues: [] };
  const outcomes = await refreshPlanConnections([
    connection("first", ["planning"]),
    connection("upload-only", ["activities"]),
    connection("second", ["planning", "activities"]),
  ], async (id) => {
    calls.push(id);
    if (id === "first") throw { code: "offline", message: "Unavailable" };
    return report;
  });
  assert.deepEqual(calls, ["first", "second"]);
  assert.equal(outcomes[0].error.code, "offline");
  assert.equal(outcomes[1].report, report);
});

test("unconnected providers make no calls and missing credentials produce an independent outcome", async () => {
  const calls = [];
  const outcomes = await refreshPlanConnections([
    connection(null, ["planning"], "disconnected"),
    connection("expired", ["planning"], "needs_sign_in"),
    connection("ready", ["planning"]),
  ], async (id) => { calls.push(id); return { issues: [] }; });
  assert.deepEqual(calls, ["ready"]);
  assert.equal(outcomes.length, 2);
  assert.equal(outcomes[0].error.code, "provider_authentication");
  assert.ok(outcomes[1].report);
});
