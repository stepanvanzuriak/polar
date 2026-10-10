import { ASYNC, bridge, http, isPolarError, payload, show, variant } from "../../runtime.js";
import { existsSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const TIMEOUT = Symbol("timeout");

export async function guard(run, timeoutMs) {
  let timer;
  const deadline = new Promise((resolve) => {
    timer = setTimeout(() => resolve(TIMEOUT), timeoutMs);
  });

  try {
    const done = Promise.resolve().then(() => run(ASYNC));
    const out = await Promise.race([done, deadline]);

    return variant(out === TIMEOUT ? "Timeout" : "Pass");
  } catch (error) {
    return outcome(error);
  } finally {
    clearTimeout(timer);
  }
}

export function clock() {
  return performance.now();
}

export function finish() {
  process.stdout.write("", () => process.exit());
}

function outcome(error) {
  const at = location(error);

  if (isPolarError(error)) {
    if (error.type === "Assert.Failed") {
      return variant("Fail", { kind: "", message: payload(error.value), at });
    }

    return variant("Error", { kind: "throws", message: show(error.value), at });
  }

  const message = error instanceof Error ? `${error.name}: ${error.message}` : String(error);

  return variant("Error", { kind: "host", message, at });
}

function location(error) {
  const frames = String(error?.stack ?? "")
    .split("\n")
    .slice(1)
    .map((line) => line.trim())
    .filter((line) => line.startsWith("at ") && !/node:|\/_polar\//.test(line));

  const found = frames.map(source).filter(Boolean);

  return found.find((at) => !at.startsWith("..")) ?? found[0] ?? "";
}

function source(frame) {
  const found = frame.match(/((?:file:\/\/)?[^\s()]+\.px):(\d+):(\d+)\)?$/);

  if (!found) return null;

  const [, file, line, column] = found;
  const path = file.startsWith("file://") ? fileURLToPath(file) : file;

  return `${relative(process.cwd(), path)}:${line}:${column}`;
}

export async function request(options) {
  const bodyless = options.method === "GET" || options.method === "HEAD";

  try {
    const response = await fetch(options.url, {
      method: options.method,
      headers: options.headers.map(({ name, value }) => [name, value]),
      body: bodyless || options.body === "" ? undefined : options.body,
      redirect: options.follow ? "follow" : "manual",
      signal: AbortSignal.timeout(options.timeout_ms),
    });
    const headers = [];

    for (const [name, value] of response.headers) {
      if (name !== "set-cookie") {
        headers.push({ name, value });
      }
    }

    for (const value of response.headers.getSetCookie()) {
      headers.push({ name: "set-cookie", value });
    }

    return {
      ok: {
        status: response.status,
        headers,
        body: await response.text(),
      },
    };
  } catch (error) {
    const timeout = error?.name === "TimeoutError";

    const message = error?.cause?.message ?? error?.message ?? String(error);

    return { err: variant(timeout ? "TimedOut" : "Network", message) };
  }
}

const dist = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const logs = new Map();
let next = 1;

export async function with_app(env, body) {
  const saved = env.map(({ name }) => [name, process.env[name]]);

  for (const { name, value } of env) {
    process.env[name] = value;
  }

  try {
    return await serve(body);
  } finally {
    restore(saved);
  }
}

async function serve(body) {
  const main = process.env.POLAR_TEST_MAIN ?? "main.js";
  const file = resolve(dist, main);
  const program = await import(pathToFileURL(file).href);

  const lines = [];
  const handlers = [];
  const client = process.env.POLAR_TEST_BROWSER_DIST;
  const bridges = join(dist, "_polar", "bridges.js");

  if (client) {
    handlers.push(http.files(client, { prefix: "/_client/" }));
  }

  if (existsSync(bridges)) {
    const loaded = await import(pathToFileURL(bridges).href);

    handlers.push(bridge.handler(loaded.bridges));
  }

  if (typeof program.router === "function") {
    handlers.push(http.handler(program.router));
  } else if (handlers.length === 0) {
    throw new Error(`\`${main}\` does not export \`router\``);
  }

  const listener = http.nodeListener(http.compose(...handlers));
  const sockets = new Set();
  const server = createServer((req, res) => {
    const started = performance.now();

    res.on("finish", () => {
      const ms = Math.round(performance.now() - started);

      lines.push(`${req.method} ${req.url} ${res.statusCode} (${ms}ms)`);
    });
    listener(req, res);
  });

  server.on("connection", (socket) => {
    sockets.add(socket);
    socket.on("close", () => sockets.delete(socket));
  });

  await new Promise((done, fail) => {
    server.once("error", fail);
    server.listen(0, "127.0.0.1", done);
  });

  const id = next++;

  logs.set(id, lines);

  try {
    const { port } = server.address();

    return await body({ url: `http://127.0.0.1:${port}`, id }, ASYNC);
  } finally {
    logs.delete(id);

    for (const socket of sockets) {
      socket.destroy();
    }

    await new Promise((done) => server.close(done));
  }
}

export function log(app) {
  return [...(logs.get(app.id) ?? [])];
}

function restore(saved) {
  for (const [name, value] of saved) {
    if (value === undefined) {
      delete process.env[name];
    } else {
      process.env[name] = value;
    }
  }
}


export async function compare_snapshot(name, text, dir, update) {
  const path = join(process.cwd(), dir, `${name}.txt`);
  const shown = relative(process.cwd(), path);
  let expected = null;

  try {
    expected = await readFile(path, "utf8");
  } catch {}

  if (expected === text) {
    return variant("Match");
  }

  if (update) {
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, text, "utf8");
    return variant("Created");
  }

  if (expected === null) {
    return variant("Missing", shown);
  }

  return variant("Differ", diff(shown, expected, text));
}

function diff(shown, expected, actual) {
  const a = expected.split("\n");
  const b = actual.split("\n");
  const out = [`snapshot ${shown} differs (- expected, + actual)`];
  const table = Array.from({ length: a.length + 1 }, () =>
    new Array(b.length + 1).fill(0),
  );

  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      table[i][j] =
        a[i] === b[j]
          ? table[i + 1][j + 1] + 1
          : Math.max(table[i + 1][j], table[i][j + 1]);
    }
  }

  let i = 0;
  let j = 0;

  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && a[i] === b[j]) {
      i++;
      j++;
    } else if (j >= b.length || (i < a.length && table[i + 1][j] >= table[i][j + 1])) {
      out.push(`- ${a[i++]}`);
    } else {
      out.push(`+ ${b[j++]}`);
    }
  }

  return out.join("\n");
}

export async function run_browser(app, name) {
  const root = process.env.POLAR_TEST_BROWSER_DIST;
  const main = process.env.POLAR_TEST_MAIN ?? "main.js";

  if (!root) {
    throw new Error(
      "no Browser build to run: list `Browser` and `Node` in the project's " +
        "`hosts` and use `Browser` in a source module",
    );
  }

  const runtime = await import(
    pathToFileURL(join(root, "_polar", "runtime.js")).href
  );
  const program = await import(pathToFileURL(resolve(root, main)).href);

  if (typeof program[name] !== "function") {
    throw new Error(`\`${main}\` for Browser does not export \`${name}\``);
  }

  runtime.bridge.configure({ base: `${app.url}/_polar` });

  const writes = [];
  const previous = globalThis.document;

  globalThis.document = fakeDocument(writes);

  try {
    await program[name](ASYNC);
  } finally {
    if (previous === undefined) {
      delete globalThis.document;
    } else {
      globalThis.document = previous;
    }
  }

  return writes;
}

function fakeDocument(writes) {
  const ids = new Map();
  const register = (html) => {
    for (const [, id] of html.matchAll(/\bid="([^"]+)"/g)) {
      ids.set(id, { innerHTML: "" });
    }
  };

  return {
    body: {
      append(element) {
        writes.push(`${element.tagName}: ${element.textContent}`);
      },
      insertAdjacentHTML(_, html) {
        register(html);
        writes.push(`html: ${html}`);
      },
    },
    createElement(tagName) {
      return { tagName, textContent: "" };
    },
    getElementById(id) {
      const element = ids.get(id);

      if (element === undefined) {
        return null;
      }

      return {
        set innerHTML(html) {
          register(html);
          writes.push(`#${id}: ${html}`);
        },
      };
    },
  };
}
