import { createHash } from "node:crypto";
import { createReadStream, createWriteStream } from "node:fs";
import { mkdir, readFile, rename, rm, stat } from "node:fs/promises";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const REPOSITORY_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const MANIFEST_PATH = path.join(
  REPOSITORY_ROOT,
  "frontend/voice/voice-models.manifest.json",
);
const BUNDLED_MODEL_ROOT = path.join(REPOSITORY_ROOT, "voice-models");
const VERIFY_ONLY = process.argv.includes("--verify-only");

const manifest = JSON.parse(await readFile(MANIFEST_PATH, "utf8"));

function modelDirectory(model) {
  return path.join(BUNDLED_MODEL_ROOT, model.bundleDirectory);
}

async function sha256(filePath) {
  const hash = createHash("sha256");
  await pipeline(createReadStream(filePath), hash);
  return hash.digest("hex");
}

async function verifyFile(model, file) {
  const filePath = path.join(modelDirectory(model), file.path);
  const details = await stat(filePath);
  if (details.size !== file.size) {
    throw new Error(`${file.path}: expected ${file.size} bytes, found ${details.size}`);
  }
  const actualHash = await sha256(filePath);
  if (actualHash !== file.sha256) {
    throw new Error(`${file.path}: expected sha256 ${file.sha256}, found ${actualHash}`);
  }
}

async function downloadMoonshineModel(model) {
  const directory = modelDirectory(model);
  await mkdir(directory, { recursive: true });
  for (const file of model.files) {
    const destination = path.join(directory, file.path);
    const partial = `${destination}.partial`;
    await mkdir(path.dirname(destination), { recursive: true });
    try {
      await verifyFile(model, file);
      continue;
    } catch {
      await rm(destination, { force: true });
    }
    const response = await fetch(`${model.sourceBaseUrl}/${file.path}`);
    if (!response.ok || !response.body) {
      throw new Error(`Moonshine download failed for ${file.path}: ${response.status}`);
    }
    await pipeline(Readable.fromWeb(response.body), createWriteStream(partial));
    await rename(partial, destination);
  }
}

if (!VERIFY_ONLY) {
  await downloadMoonshineModel(manifest.moonshine);
  await downloadMoonshineModel(manifest.embedding);
}

for (const model of [manifest.moonshine, manifest.embedding]) {
  for (const file of model.files) await verifyFile(model, file);
}

const totalBytes = [manifest.moonshine, manifest.embedding]
  .flatMap((model) => model.files)
  .reduce((total, file) => total + file.size, 0);
console.log(`Verified ${totalBytes} bytes of pinned voice-model assets.`);
