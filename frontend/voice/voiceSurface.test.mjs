import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { before, test } from "node:test";
import { transformWithEsbuild } from "vite";

let normalizeVoiceTranscript;

before(async () => {
  const sourceUrl = new URL("./voiceSurface.ts", import.meta.url);
  const source = await readFile(sourceUrl, "utf8");
  const transformed = await transformWithEsbuild(source, sourceUrl.pathname, {
    format: "esm",
    loader: "ts",
    target: "es2022",
  });
  const moduleUrl = `data:text/javascript;base64,${Buffer.from(transformed.code).toString("base64")}`;
  ({ normalizeVoiceTranscript } = await import(moduleUrl));
});

test("normalizes punctuation without inventing a prefix", () => {
  assert.equal(normalizeVoiceTranscript("  Set intensity—to 95%! "), "set intensity to 95%");
});
