import assert from "node:assert/strict";
import { build } from "esbuild";
import { before, beforeEach, test } from "node:test";

let WorkerMicTranscriber;
class Host {
  static instances = [];
  lines = [];
  constructor() { Host.instances.push(this); }
  async loadTranscriber() {}
  async createStream() {}
  setListener(_, listener) { this.listener = listener; }
  async start() {}
  addAudio() {}
  async stop() {
    this.stoppedAfterMic = globalThis.testTrack.stopped;
    for (const line of this.lines) this.listener?.onLineCompleted({ line });
  }
  async closeStream() { this.listener = null; }
  close() {}
}

before(async () => {
  globalThis.TestSttHost = Host;
  const result = await build({
    entryPoints: [new URL("./workerMicTranscriber.ts", import.meta.url).pathname],
    bundle: true, format: "esm", platform: "browser", write: false,
    plugins: [{ name: "fake-stt-boundaries", setup(build) {
      build.onResolve({ filter: /^@moonshine-ai\/moonshine-wasm/ }, ({ path }) => ({ path, namespace: "fake" }));
      build.onResolve({ filter: /^\.\.\/trace$/ }, ({ path }) => ({ path, namespace: "fake" }));
      build.onLoad({ filter: /.*/, namespace: "fake" }, ({ path }) => ({ contents:
        path === "../trace" ? "export const traceEvent = () => {};" :
        path.includes("?") ? 'export default "fake-asset";' :
        "export const SttWorkerHost = globalThis.TestSttHost; export const ModelArch = { SmallStreaming: 0 };",
      }));
    } }],
  });
  ({ WorkerMicTranscriber } = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`));
});

beforeEach(() => {
  globalThis.window = { location: { href: "https://qa.invalid/" } };
  const track = { stopped: false, stop() { this.stopped = true; }, addEventListener() {}, getSettings: () => ({}) };
  globalThis.testTrack = track;
  const stream = { getTracks: () => [track], getAudioTracks: () => [track] };
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: { mediaDevices: {
    getUserMedia: async () => stream, addEventListener() {}, removeEventListener() {},
  } } });
  globalThis.AudioContext = class {
    state = "running";
    sampleRate = 48000;
    createMediaStreamSource() { return { connect() {}, disconnect() {} }; }
    createScriptProcessor() { return { connect() {}, disconnect() {} }; }
    addEventListener() {}
    removeEventListener() {}
    async close() {}
  };
});

function microphone() {
  const completed = [];
  const mic = new WorkerMicTranscriber("https://qa.invalid/models/", {
    onText() {}, onLine: (line) => completed.push(line.text), onError: (e) => { throw e; },
    onCaptureDiscontinuity() {}, onProgress() {},
  });
  return { mic, completed, host: Host.instances.at(-1) };
}

test("PTT release includes trailing transcription and deduplicates line revisions", async () => {
  const { mic, host } = microphone();
  await mic.start();
  host.listener.onLineTextChanged({ line: { id: "1", startTime: 0, text: "set intensity" } });
  host.lines = [{ id: "1", startTime: 0, text: "set intensity to ninety" }, { id: "2", startTime: 1, text: "percent" }];
  assert.equal(await mic.finish(), "set intensity to ninety percent");
  assert.equal(host.stoppedAfterMic, true);
  assert.equal(mic.mediaStream, undefined);
});

test("cancel discards final callbacks instead of submitting them", async () => {
  const { mic, host, completed } = microphone();
  await mic.start();
  host.lines = [{ id: "1", startTime: 0, text: "skip" }];
  mic.invalidateCapture();
  await mic.stop();
  assert.deepEqual(completed, []);
  assert.equal(testTrack.stopped, true);
});

test("cancel while microphone permission is pending closes the late stream", async () => {
  const { mic } = microphone();
  let resolveMic, requested;
  const pending = new Promise((resolve) => { requested = resolve; });
  navigator.mediaDevices.getUserMedia = () => {
    requested();
    return new Promise((resolve) => { resolveMic = resolve; });
  };
  const starting = mic.start();
  await pending;
  mic.invalidateCapture();
  resolveMic({ getTracks: () => [testTrack], getAudioTracks: () => [testTrack] });
  await starting;
  assert.equal(testTrack.stopped, true);
  assert.equal(mic.mediaStream, undefined);
  assert.equal(await mic.finish(), "");
});

test("a silent hold returns no command text", async () => {
  const { mic } = microphone();
  await mic.start();
  assert.equal(await mic.finish(), "");
});
