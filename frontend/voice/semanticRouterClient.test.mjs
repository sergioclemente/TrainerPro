import assert from "node:assert/strict";
import { build } from "esbuild";
import { before, beforeEach, test } from "node:test";

let SemanticRouterClient;

class FakeWorker {
  static instances = [];

  messages = [];
  terminated = false;
  onmessage = null;
  onerror = null;

  constructor() {
    FakeWorker.instances.push(this);
  }

  postMessage(message) {
    this.messages.push(message);
  }

  terminate() {
    this.terminated = true;
  }

  respond(response) {
    this.onmessage?.({ data: response });
  }
}

async function loadClientModule() {
  const sourceUrl = new URL("./semanticRouterClient.ts", import.meta.url);
  const result = await build({
    entryPoints: [sourceUrl.pathname],
    bundle: true,
    format: "esm",
    platform: "browser",
    target: "es2022",
    write: false,
  });
  const bundled = result.outputFiles[0].text.replaceAll(
    "import.meta.url",
    JSON.stringify(sourceUrl.href),
  );
  const moduleUrl = `data:text/javascript;base64,${Buffer.from(bundled).toString("base64")}`;
  return import(moduleUrl);
}

function loadedResponse(requestId) {
  return { type: "loaded", requestId, modelLoadMs: 1, totalDurationMs: 1 };
}

function routeResponse(requestId) {
  return {
    type: "result",
    requestId,
    transcript: "pause",
    threshold: 0.7,
    intentId: "pause",
    matchedPhrase: "pause",
    score: 1,
    rejection: null,
    embeddingDurationMs: 1,
    totalDurationMs: 1,
  };
}

async function loadedClient(options) {
  const client = new SemanticRouterClient(() => {}, options);
  const worker = FakeWorker.instances.at(-1);
  const loading = client.load("asset://voice-model/");
  worker.respond(loadedResponse(worker.messages[0].requestId));
  await loading;
  return { client, worker };
}

before(async () => {
  globalThis.Worker = FakeWorker;
  ({ SemanticRouterClient } = await loadClientModule());
});

beforeEach(() => {
  FakeWorker.instances = [];
});

test("routes one request through the loaded worker", async () => {
  const { client, worker } = await loadedClient();
  const routing = client.route(
    "pause",
    [{ id: "pause", phrases: ["pause"] }],
    0.7,
  );
  const request = worker.messages.at(-1);
  worker.respond(routeResponse(request.requestId));

  assert.equal((await routing).intentId, "pause");
  assert.equal(worker.terminated, false);
  client.close();
});

test("rejects a second request while semantic routing is in flight", async () => {
  const { client, worker } = await loadedClient();
  const first = client.route("pause", [{ id: "pause", phrases: ["pause"] }], 0.7);

  await assert.rejects(
    client.route("resume", [{ id: "resume", phrases: ["resume"] }], 0.7),
    /worker is busy/,
  );

  const request = worker.messages.at(-1);
  worker.respond(routeResponse(request.requestId));
  await first;
  client.close();
});

test("a routing timeout terminates the worker and discards late results", async () => {
  const { client, worker } = await loadedClient({ routeTimeoutMs: 5 });
  const routing = client.route(
    "pause",
    [{ id: "pause", phrases: ["pause"] }],
    0.7,
  );
  const request = worker.messages.at(-1);

  await assert.rejects(routing, /timed out after 5 ms/);
  assert.equal(worker.terminated, true);

  worker.respond(routeResponse(request.requestId));
  await assert.rejects(
    client.route("pause", [{ id: "pause", phrases: ["pause"] }], 0.7),
    /not loaded|closed/,
  );
});

test("surfaces a packaged command-model load failure without becoming loaded", async () => {
  const client = new SemanticRouterClient(() => {});
  const worker = FakeWorker.instances.at(-1);
  const loading = client.load("asset://missing-voice-model/");
  const request = worker.messages[0];
  worker.respond({
    type: "error",
    requestId: request.requestId,
    operation: "load",
    message: "model data is missing or invalid",
  });

  await assert.rejects(loading, /model data is missing or invalid/);
  await assert.rejects(
    client.route("pause", [{ id: "pause", phrases: ["pause"] }], 0.7),
    /not loaded/,
  );
  client.close();
});
