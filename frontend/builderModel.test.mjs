import assert from "node:assert/strict";
import { build } from "esbuild";
import { before, test } from "node:test";

let model;
before(async () => {
  const result = await build({
    entryPoints: [new URL("./builder/model.ts", import.meta.url).pathname],
    bundle: true, format: "esm", platform: "node", write: false,
  });
  model = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`);
});

const stored = {
  name: "Over-unders",
  description: "Three sets",
  nodes: [
    { kind: "ramp", duration_s: 600, start_pct: 50, end_pct: 75, cadence_rpm: null },
    {
      kind: "repeat",
      count: 3,
      children: [
        { kind: "simple", duration_s: 120, power_pct: 105, cadence_rpm: 95 },
        { kind: "simple", duration_s: 60, power_pct: 85, cadence_rpm: null },
      ],
    },
    { kind: "simple", duration_s: 300, power_pct: 55, cadence_rpm: null },
  ],
};

test("a stored draft opens in the editor and saves back unchanged", () => {
  const nodes = model.fromDraft(stored.nodes);
  assert.deepEqual(model.toDraft(stored.name, stored.description, nodes), stored);
});

test("reopened nodes get distinct editor ids so selection and drag stay addressable", () => {
  const nodes = model.fromDraft(stored.nodes);
  const ids = nodes.flatMap((n) => (n.kind === "repeat" ? [n.id, ...n.children.map((c) => c.id)] : [n.id]));
  assert.equal(new Set(ids).size, ids.length);
  assert.ok(ids.every((id) => typeof id === "string" && id.length > 0));
});
