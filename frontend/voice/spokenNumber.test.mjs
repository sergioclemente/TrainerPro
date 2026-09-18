import assert from "node:assert/strict";
import { build } from "esbuild";
import { before, test } from "node:test";

let createIntegerSlotParser;

before(async () => {
  const entry = new URL("./spokenNumber.ts", import.meta.url).pathname;
  const result = await build({
    entryPoints: [entry],
    bundle: true,
    format: "esm",
    platform: "node",
    target: "es2022",
    write: false,
  });
  const moduleUrl = `data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`;
  ({ createIntegerSlotParser } = await import(moduleUrl));
});

test("parses only values admitted by a finite domain", () => {
  const watts = createIntegerSlotParser({ min: 100, max: 800, step: 5 });
  assert.equal(watts.parse("set power to 275 watts"), 275);
  assert.equal(watts.parse("set power to two hundred and seventy five watts"), 275);
  assert.equal(watts.parse("set power to two seventy five watts"), 275);
  assert.equal(watts.parse("set power to two oh five watts"), 205);
  assert.equal(watts.parse("set power to 277 watts"), null);
  assert.equal(watts.parse("set power to nine hundred watts"), null);
});

test("supports compact spoken percentages without general arithmetic", () => {
  const percent = createIntegerSlotParser({ min: 50, max: 150, step: 1 });
  assert.equal(percent.parse("set intensity to ninety five percent"), 95);
  assert.equal(percent.parse("set intensity to one twenty percent"), 120);
  assert.equal(percent.parse("set intensity to one hundred and fifty percent"), 150);
  assert.equal(percent.parse("set intensity to forty nine percent"), null);
  assert.equal(percent.parse("set intensity to one hundred sixty percent"), null);
});

test("rejects malformed domains when they are declared", () => {
  assert.throws(
    () => createIntegerSlotParser({ min: 800, max: 100, step: 5 }),
    /ordered range/,
  );
  assert.throws(
    () => createIntegerSlotParser({ min: 100, max: 1_000, step: 5 }),
    /between 0 and 999/,
  );
});
