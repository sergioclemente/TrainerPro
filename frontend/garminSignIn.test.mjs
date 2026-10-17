import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import { build } from "esbuild";
import { before, test } from "node:test";

let GarminSignIn;
let hooks;
let ipc;
before(async () => {
  const result = await build({
    stdin: {
      contents: `export { default as GarminSignIn } from "./components/provider-auth/GarminSignIn";
        export * as hooks from "react";
        export { ipc } from "./ipc";`,
      resolveDir: new URL(".", import.meta.url).pathname,
    },
    bundle: true, format: "esm", platform: "node", write: false,
    jsx: "automatic",
    plugins: [{
      name: "sign-in-boundaries",
      setup(build) {
        build.onResolve({ filter: /^react$/ }, () => ({ path: "react", namespace: "test" }));
        build.onResolve({ filter: /^react\/jsx-runtime$/ }, () => ({ path: "jsx", namespace: "test" }));
        build.onResolve({ filter: /\/ipc$|^\.\/ipc$/ }, () => ({ path: "ipc", namespace: "test" }));
        build.onLoad({ filter: /.*/, namespace: "test" }, ({ path }) => ({ contents: path === "ipc"
          ? "export const ipc = {};"
          : path === "jsx"
          ? "export function jsx(type, props) { return { type, props }; } export const jsxs = jsx;"
          : `let slots = [], cursor = 0;
            export function reset() { slots = []; cursor = 0; }
            export function render(component, props) { cursor = 0; return component(props); }
            export function useState(initial) {
              const index = cursor++;
              if (!(index in slots)) slots[index] = initial;
              return [slots[index], value => { slots[index] = value; }];
            }
            export function useRef(initial) {
              const index = cursor++;
              return slots[index] ??= { current: initial };
            }
            export function useEffect() {}`,
        }));
      },
    }],
  });
  ({ GarminSignIn, hooks, ipc } = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`));
});

function elements(tree, predicate) {
  if (!tree || typeof tree !== "object") return [];
  return [
    ...(predicate(tree) ? [tree] : []),
    ...[tree.props?.children].flat().flatMap(child => elements(child, predicate)),
  ];
}

async function withCrypto(t, crypto, action) {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, "crypto");
  Object.defineProperty(globalThis, "crypto", { configurable: true, value: crypto });
  t.after(() => descriptor
    ? Object.defineProperty(globalThis, "crypto", descriptor)
    : delete globalThis.crypto);
  hooks.reset();
  let connected = 0;
  const props = { disabled: false, onConnected: async () => { connected++; } };
  const render = () => hooks.render(GarminSignIn, props);
  const button = () => elements(render(), item => item.type === "button")[0];
  const click = async () => { button().props.onClick(); await new Promise(setImmediate); };
  await action({ render, button, click, connected: () => connected });
}

test("sign-in without randomUUID sends valid distinct v4 IDs and cancels the matching attempt", async (t) => {
  await withCrypto(t, { getRandomValues: webcrypto.getRandomValues.bind(webcrypto) }, async ({ render, button, click }) => {
    const attempts = [];
    let finish;
    ipc.connectGarmin = (id) => { attempts.push(id); return new Promise(resolve => { finish = resolve; }); };
    ipc.cancelGarminSignIn = async (id) => {
      assert.equal(id, attempts.at(-1));
      finish({ status: "cancelled" });
    };
    for (let run = 0; run < 2; run++) {
      await click();
      assert.equal(button().props.disabled, true);
      assert.match(attempts[run], /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
      elements(render(), item => item.type === "button")[1].props.onClick();
      await new Promise(setImmediate);
      assert.equal(button().props.disabled, false);
    }
    assert.notEqual(attempts[0], attempts[1]);
  });
});

test("ID generation failure shows an error and allows a successful retry", async (t) => {
  let fail = true;
  await withCrypto(t, {
    getRandomValues(bytes) {
      if (fail) throw new Error("Random source unavailable");
      return webcrypto.getRandomValues(bytes);
    },
  }, async ({ render, button, click, connected }) => {
    let calls = 0;
    ipc.connectGarmin = async () => { calls++; return { status: "connected" }; };
    await click();
    assert.equal(calls, 0);
    assert.equal(button().props.disabled, false);
    assert.equal(elements(render(), item => item.props.role === "alert")[0].props.children, "Random source unavailable");
    fail = false;
    await click();
    assert.equal(calls, 1);
    assert.equal(connected(), 1);
    assert.equal(button().props.disabled, false);
    assert.equal(elements(render(), item => item.props.role === "alert").length, 0);
  });
});

test("sign-in uses native randomUUID when available", async (t) => {
  const id = "8c4c701b-2973-4ba4-a67d-ff8e3dfac2e5";
  await withCrypto(t, { randomUUID: () => id }, async ({ button, click, connected }) => {
    ipc.connectGarmin = async (attemptId) => {
      assert.equal(attemptId, id);
      return { status: "connected" };
    };
    await click();
    assert.equal(connected(), 1);
    assert.equal(button().props.disabled, false);
  });
});
