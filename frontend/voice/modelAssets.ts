import { convertFileSrc } from "@tauri-apps/api/core";
import { join, resourceDir } from "@tauri-apps/api/path";
import voiceModelManifest from "./voice-models.manifest.json";

export interface VoiceModelUrls {
  moonshine: string;
  embedding: string;
}

let urlsPromise: Promise<VoiceModelUrls> | null = null;

function withTrailingSlash(url: string): string {
  return url.endsWith("/") ? url : `${url}/`;
}

export function resolveVoiceModelUrls(): Promise<VoiceModelUrls> {
  urlsPromise ??= resourceDir().then(async (resources) => ({
    moonshine: withTrailingSlash(
      convertFileSrc(
        await join(resources, "voice-models", voiceModelManifest.moonshine.bundleDirectory),
      ),
    ),
    embedding: withTrailingSlash(
      convertFileSrc(
        await join(resources, "voice-models", voiceModelManifest.embedding.bundleDirectory),
      ),
    ),
  }));
  return urlsPromise;
}
