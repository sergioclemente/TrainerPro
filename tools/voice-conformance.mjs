import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  EmbeddingModel,
  EmbeddingModelArch,
} from "@moonshine-ai/moonshine-wasm";
import { build } from "esbuild";

const REPOSITORY_ROOT = fileURLToPath(new URL("..", import.meta.url));
const MANIFEST_PATH = path.join(
  REPOSITORY_ROOT,
  "frontend/voice/voice-models.manifest.json",
);
const CORPUS_PATH = path.join(
  REPOSITORY_ROOT,
  "frontend/screens/Player.voice.conformance.json",
);
const PLAYER_VOICE_PATH = path.join(REPOSITORY_ROOT, "frontend/screens/Player.voice.ts");
const VOICE_CONTROLS_PATH = path.join(REPOSITORY_ROOT, "frontend/voice/voiceControls.ts");
const VOICE_ROUTING_PATH = path.join(REPOSITORY_ROOT, "frontend/voice/voiceRouting.ts");
const MATCHER_PATH = path.join(REPOSITORY_ROOT, "frontend/voice/semanticMatcher.ts");
const VALID_PHASES = ["ready", "riding", "paused"];
const verbose = process.argv.includes("--verbose");

async function importBundled(entryPoint) {
  const result = await build({
    entryPoints: [entryPoint],
    bundle: true,
    format: "esm",
    platform: "node",
    target: "es2022",
    write: false,
  });
  const source = result.outputFiles[0].text;
  const moduleUrl = `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`;
  return import(moduleUrl);
}

function player(phase) {
  return { phase, intensity: 1, erg_enabled: true };
}

function createSurfaceHarness(playerVoiceModule, voiceControlsModule, voiceRoutingModule) {
  let currentPlayer = player("ready");
  let commandsSuspended = false;
  const noOp = async () => undefined;
  const surface = playerVoiceModule.createPlayerVoiceSurface({
    getPlayer: () => currentPlayer,
    commands: {
      startRide: noOp,
      pauseRide: noOp,
      resumeRide: noOp,
      skipSegment: noOp,
      setIntensity: noOp,
      setErg: noOp,
    },
  });
  const controls = voiceControlsModule.createVoiceControlSurface({
    getCommandsSuspended: () => commandsSuspended,
    setCommandsSuspended: (suspended) => {
      commandsSuspended = suspended;
    },
  });
  return {
    surface,
    catalog: voiceRoutingModule.voiceRoutingCatalog(surface, controls),
    setContext: (phase, suspended = false) => {
      currentPlayer = player(phase);
      commandsSuspended = suspended;
    },
    currentIntentGroups: () => voiceRoutingModule.currentVoiceIntentGroups(
      surface,
      controls,
      commandsSuspended,
    ),
    prepareCommand: (match) => voiceRoutingModule.prepareRoutedVoiceCommand(
      surface,
      controls,
      commandsSuspended,
      match,
    ),
  };
}

function canonicalCases(harness) {
  const contexts = VALID_PHASES.flatMap((phase) => [
    { phase, commandsSuspended: false },
    { phase, commandsSuspended: true },
  ]);
  return harness.catalog.flatMap((group) => {
    return group.phrases.map((transcript) => {
      const context = contexts.find(({ phase, commandsSuspended }) => {
        harness.setContext(phase, commandsSuspended);
        return harness.currentIntentGroups().some(({ id }) => id === group.id);
      });
      if (!context) throw new Error(`Command ${group.id} has no active voice context`);
      return {
        name: `canonical ${group.id}: ${transcript}`,
        ...context,
        transcript,
        expectedIntent: group.id,
        expectedCommand: true,
        source: "canonical",
      };
    });
  });
}

function percentile(values, fraction) {
  if (values.length === 0) return 0;
  const ordered = [...values].sort((left, right) => left - right);
  return ordered[Math.min(ordered.length - 1, Math.floor(ordered.length * fraction))];
}

function formatMatch(result) {
  if (!result.bestIntent) return "no candidates";
  const accepted = result.actualIntent ?? "rejected";
  return `${accepted} (best ${result.bestIntent} ${result.score.toFixed(3)} via “${result.matchedPhrase}”)`;
}

async function evaluate(testCase, harness, matcher) {
  harness.setContext(testCase.phase, testCase.commandsSuspended ?? false);
  const { surface } = harness;
  const transcriptPreparation = surface.prepareTranscript?.(testCase.transcript) ?? {
    kind: "route",
    transcript: testCase.transcript,
  };
  if (transcriptPreparation.kind === "rejected") {
    return {
      ...testCase,
      actualIntent: null,
      commandPrepared: false,
      label: null,
      bestIntent: null,
      matchedPhrase: null,
      score: null,
      embeddingDurationMs: 0,
    };
  }

  const groups = harness.currentIntentGroups();
  if (groups.length === 0) {
    return {
      ...testCase,
      actualIntent: null,
      commandPrepared: false,
      label: null,
      bestIntent: null,
      matchedPhrase: null,
      score: null,
      embeddingDurationMs: 0,
    };
  }
  const { match, embeddingDurationMs } = matcher.bestMatch(
    transcriptPreparation.transcript,
    harness.catalog,
  );
  const acceptedMatch = match && match.score >= surface.threshold ? match : null;
  const prepared = acceptedMatch
    ? harness.prepareCommand({
        intentId: acceptedMatch.intentId,
        transcript: transcriptPreparation.transcript,
        matchedPhrase: acceptedMatch.phrase,
        score: acceptedMatch.score,
      })
    : null;

  return {
    ...testCase,
    actualIntent: acceptedMatch?.intentId ?? null,
    commandPrepared: prepared?.kind === "command",
    label: prepared?.kind === "command" ? prepared.label : null,
    bestIntent: match?.intentId ?? null,
    matchedPhrase: match?.phrase ?? null,
    score: match?.score ?? null,
    embeddingDurationMs,
  };
}

function casePassed(result) {
  const expectedCommand = result.expectedCommand ?? result.expectedIntent !== null;
  const intentMatches = result.expectedIntent === null
    ? !result.commandPrepared
    : result.actualIntent === result.expectedIntent;
  return intentMatches &&
    result.commandPrepared === expectedCommand &&
    (result.expectedLabel === undefined || result.label === result.expectedLabel);
}

async function loadEmbeddingModel(manifest) {
  const modelRoot = path.join(
    REPOSITORY_ROOT,
    "voice-models",
    manifest.embedding.bundleDirectory,
  );
  const files = Object.fromEntries(
    manifest.embedding.files.map(({ path: file }) => [file, path.join(modelRoot, file)]),
  );
  const downloader = {
    async downloadNamedFiles(namedFiles) {
      const entries = namedFiles instanceof Map ? [...namedFiles] : Object.entries(namedFiles);
      return new Map(await Promise.all(entries.map(async ([name, file]) => [
        name,
        new Uint8Array(await readFile(file)),
      ])));
    },
  };
  const require = createRequire(import.meta.url);
  const wasmPath = require.resolve("@moonshine-ai/moonshine-wasm/moonshine.wasm");
  return EmbeddingModel.loadFromUrls(files, {
    modelArch: EmbeddingModelArch.Gemma300M,
    variant: manifest.embedding.variant,
    downloader,
    moduleOptions: {
      locateFile: (file) => file.endsWith(".wasm") ? wasmPath : file,
    },
  });
}

async function main() {
  const [
    manifest,
    corpus,
    playerVoiceModule,
    voiceControlsModule,
    voiceRoutingModule,
    matcherModule,
  ] = await Promise.all([
    readFile(MANIFEST_PATH, "utf8").then(JSON.parse),
    readFile(CORPUS_PATH, "utf8").then(JSON.parse),
    importBundled(PLAYER_VOICE_PATH),
    importBundled(VOICE_CONTROLS_PATH),
    importBundled(VOICE_ROUTING_PATH),
    importBundled(MATCHER_PATH),
  ]);
  if (corpus.schemaVersion !== 1) {
    throw new Error(`Unsupported voice conformance schema ${corpus.schemaVersion}`);
  }

  const harness = createSurfaceHarness(
    playerVoiceModule,
    voiceControlsModule,
    voiceRoutingModule,
  );
  const cases = [
    ...canonicalCases(harness),
    ...corpus.cases.map((testCase) => ({ ...testCase, source: "curated" })),
  ];

  const rssBefore = process.memoryUsage().rss;
  const loadStartedAt = performance.now();
  const model = await loadEmbeddingModel(manifest);
  const modelLoadMs = performance.now() - loadStartedAt;
  let failureCount = 0;
  try {
    const matcher = new matcherModule.SemanticMatcher(model);
    const warmupStartedAt = performance.now();
    matcher.prepare(harness.catalog);
    const warmupMs = performance.now() - warmupStartedAt;

    const results = [];
    for (const testCase of cases) results.push(await evaluate(testCase, harness, matcher));
    const failures = results.filter((result) => !casePassed(result));
    failureCount = failures.length;
    const durations = results.map(({ embeddingDurationMs }) => embeddingDurationMs);

    for (const result of results) {
      if (!verbose && casePassed(result)) continue;
      const status = casePassed(result) ? "PASS" : "FAIL";
      const expected = result.expectedIntent ?? "reject";
      console.log(`${status} [${result.phase}] ${result.name}`);
      console.log(`  “${result.transcript}”`);
      console.log(`  expected ${expected}; got ${formatMatch(result)}`);
      if (result.expectedLabel !== undefined || result.label !== null) {
        console.log(`  label ${result.label ?? "none"}`);
      }
    }

    const canonicalCount = results.filter(({ source }) => source === "canonical").length;
    const rssAfter = process.memoryUsage().rss;
    console.log(
      `Voice conformance: ${results.length - failures.length}/${results.length} passed ` +
      `(${canonicalCount} canonical, ${results.length - canonicalCount} curated)`,
    );
    console.log(
      `Model load ${modelLoadMs.toFixed(0)} ms; catalog warmup ${warmupMs.toFixed(0)} ms; ` +
      `route median ${percentile(durations, 0.5).toFixed(0)} ms; ` +
      `p95 ${percentile(durations, 0.95).toFixed(0)} ms; ` +
      `RSS +${((rssAfter - rssBefore) / 1024 / 1024).toFixed(0)} MiB`,
    );
  } finally {
    model.close();
  }
  if (failureCount > 0) process.exit(1);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack : error);
  process.exitCode = 1;
});
