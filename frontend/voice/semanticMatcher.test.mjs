import assert from "node:assert/strict";
import { build } from "esbuild";
import { before, test } from "node:test";

let SemanticMatcher;

before(async () => {
  const entry = new URL("./semanticMatcher.ts", import.meta.url).pathname;
  const result = await build({
    entryPoints: [entry],
    bundle: true,
    format: "esm",
    platform: "node",
    target: "es2022",
    write: false,
  });
  const moduleUrl = `data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`;
  ({ SemanticMatcher } = await import(moduleUrl));
});

test("prepares phrases once and returns the highest-scoring intent", () => {
  const embeddings = new Map([
    ["start", new Float32Array([1, 0])],
    ["pause", new Float32Array([0, 1])],
    ["please hold", new Float32Array([0.1, 0.9])],
  ]);
  const embedded = [];
  const model = {
    calculateEmbedding: (sentence) => {
      embedded.push(sentence);
      return embeddings.get(sentence) ?? new Float32Array([0, 0]);
    },
    distance: (left, right) => left[0] * right[0] + left[1] * right[1],
  };
  const groups = [
    { id: "start", phrases: ["start"] },
    { id: "pause", phrases: ["pause", "pause"] },
  ];
  const matcher = new SemanticMatcher(model);

  matcher.prepare(groups);
  matcher.prepare(groups);
  assert.equal(matcher.cachedPhraseCount, 2);
  assert.deepEqual(embedded, ["start", "pause"]);

  const result = matcher.bestMatch("  Please—hold! ", groups);
  assert.equal(result.match.intentId, "pause");
  assert.equal(result.match.phrase, "pause");
  assert.deepEqual(embedded, ["start", "pause", "please hold"]);
});
