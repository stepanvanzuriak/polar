const JsString = globalThis.String;

/**
 * A readable rendering of any Polar value, for debugging.
 * @param {unknown} x
 * @returns {string}
 */
export function show(x) {
  if (typeof x === "string") {
    return JSON.stringify(x);
  }

  if (typeof x === "function") {
    return "<function>";
  }

  if (x === null || typeof x !== "object") {
    return JsString(x);
  }

  if (isVariant(x)) {
    const shown = args(x).map(show);

    return !shown.length ? x.$ : `${x.$}(${shown.join(", ")})`;
  }

  const fields = Object.entries(x).map(([k, v]) => `${k}: ${show(v)}`);

  return !fields.length ? "{}" : `{ ${fields.join(", ")} }`;
}

/**
 * How Polar values look in JS: a variant value is an object whose `$` is the
 * constructor name and whose arguments are `_0`, `_1`, …, so `Some(5)` is
 * `{ $: "Some", _0: 5 }` and `None` is `{ $: "None" }`. A `List` is a chain of
 * `Cons(head, tail)` ending in `Nil`. The helpers below are the only code in
 * this file that touches that encoding directly.
 */

/**
 * `variant("Some", 5)` is `Some(5)`.
 * @param {string} name
 * @param {...unknown} args
 * @returns {object}
 */
export function variant(name, ...values) {
  const value = { $: name };

  values.forEach((arg, i) => {
    value[`_${i}`] = arg;
  });

  return value;
}

/**
 * @param {object} value
 * @returns {boolean} whether `value` is a variant value rather than a record
 */
function isVariant(value) {
  return typeof value.$ === "string";
}

/**
 * All arguments of a variant value: `args(Pair(1, 2))` is `[1, 2]`.
 * @param {object} value
 * @returns {unknown[]}
 */
function args(value) {
  const out = [];

  for (let i = 0; `_${i}` in value; i++) {
    out.push(value[`_${i}`]);
  }

  return out;
}

/**
 * The first argument of a variant value: `payload(Some(5))` is `5`.
 * @param {object} value
 * @returns {unknown}
 */
export function payload(value) {
  return value._0;
}

const some = (x) => variant("Some", x);
const none = () => variant("None");
const ok = (x) => variant("Ok", x);
const err = (e) => variant("Err", e);

/**
 * Walks a `List` from the front: `for (const x of each(list)) …`.
 * @param {object} list
 * @returns {Generator<unknown>}
 */
function* each(list) {
  for (let rest = list; rest.$ === "Cons"; rest = rest._1) {
    yield rest._0;
  }
}

/**
 * Builds a `List` from `items`, ending in `tail`: `list([1, 2])` is
 * `Cons(1, Cons(2, Nil))`. Called by generated code for `[a, b, ..rest]`.
 * @param {unknown[]} items
 * @param {unknown} [tail]
 * @returns {object}
 */
export function list(items, tail = variant("Nil")) {
  let acc = tail;

  for (let i = items.length - 1; i >= 0; i--) {
    acc = variant("Cons", items[i], acc);
  }

  return acc;
}

export const ASYNC = Symbol.for("polar.async");

export function poly(arity, sync, suspending) {
  return (...args) =>
    args[arity] === ASYNC ? suspending(...args) : sync(...args);
}

function toJs(shape, value) {
  if (shape === null || shape === "unit") {
    return value;
  }

  if (Array.isArray(shape)) {
    const [kind, inner, error] = shape;

    if (kind === "option") {
      return value.$ === "Some" ? toJs(inner, payload(value)) : null;
    }

    if (kind === "result") {
      return value.$ === "Ok"
        ? { ok: toJs(inner, payload(value)) }
        : { err: toJs(error, payload(value)) };
    }

    return [...each(value)].map((x) => toJs(inner, x));
  }

  const out = { ...value };

  for (const [key, inner] of Object.entries(shape)) {
    out[key] = toJs(inner, value[key]);
  }

  return out;
}

function fromJs(shape, value, label) {
  if (shape === null) {
    return value;
  }

  if (shape === "unit") {
    return {};
  }

  if (Array.isArray(shape)) {
    const [kind, inner, error] = shape;

    if (kind === "option") {
      return value == null ? none() : some(fromJs(inner, value, label));
    }

    if (kind === "result") {
      if (typeof value === "object" && value !== null) {
        if (Object.hasOwn(value, "ok")) {
          return ok(fromJs(inner, value.ok, label));
        }

        if (Object.hasOwn(value, "err")) {
          return err(fromJs(error, value.err, label));
        }
      }

      throw new TypeError(
        `\`${label}\` returned ${describe(value)} where a Result was expected; return { ok: value } or { err: error }`,
      );
    }

    return list(Array.from(value, (x) => fromJs(inner, x, label)));
  }

  const out = { ...value };

  for (const [key, inner] of Object.entries(shape)) {
    out[key] = fromJs(inner, value[key], label);
  }

  return out;
}

function describe(value) {
  try {
    return JSON.stringify(value) ?? String(value);
  } catch {
    return String(value);
  }
}

export function extern(f, params, ret, label = f.name) {
  return (...args) => {
    const out = f(...params.map((shape, i) => toJs(shape, args[i])));

    return typeof out?.then === "function"
      ? out.then((value) => fromJs(ret, value, label))
      : fromJs(ret, out, label);
  };
}

/** Thrown when no `match` arm matches. */
export class PolarMatchError extends Error {}

/**
 * Marks a Polar error. It is a registered symbol, so an error thrown by an
 * extern that can't import this file is still recognised.
 */
const BRAND = Symbol.for("polar.error");

/**
 * A Polar error raised by `throw`. `type` names the error's type as
 * `Module.Type` (`App.Missing`), which is what a `catch` matches on, and
 * `value` is the thrown value itself.
 *
 * An extern that declares `/ {Throws<E>}` must throw a Polar error for `E`:
 * `raise("App.Missing", { $: "Missing", _0: key })` if it imports this file,
 * or else any object carrying the brand,
 * `{ [Symbol.for("polar.error")]: true, type: "App.Missing", value }`.
 * The compiler can't check that the JavaScript keeps this promise.
 */
export class PolarError extends Error {
  /**
   * @param {string} type
   * @param {unknown} value
   */
  constructor(type, value) {
    super(`uncaught error: ${show(value)}`);
    this.name = "PolarError";
    this.type = type;
    this.value = value;
    this[BRAND] = true;
  }
}

/**
 * Whether `e` is a Polar error, from this runtime or branded by an extern.
 * @param {unknown} e
 * @returns {boolean}
 */
export function isPolarError(e) {
  return typeof e === "object" && e !== null && e[BRAND] === true;
}

/**
 * Called by generated code for `throw`. Never returns.
 * @param {string} type  the error's type, as `Module.Type`
 * @param {unknown} value
 * @returns {never}
 */
export function raise(type, value) {
  throw new PolarError(type, value);
}

/**
 * Whether a `catch` handling `types` catches `e`. Anything that isn't a
 * `PolarError` (a bug in an extern, a failed `match`) is never caught.
 * @param {unknown} e
 * @param {string[]} types
 * @returns {boolean}
 */
export function handles(e, types) {
  return isPolarError(e) && types.includes(e.type);
}

/**
 * What happens to a Polar error nothing caught: by default, print
 * `uncaught error: <value>` to stderr and fail the process.
 * @type {(e: PolarError) => unknown}
 */
let uncaught = (e) => {
  console.error(`uncaught error: ${show(e.value)}`);

  if (globalThis.process) {
    globalThis.process.exitCode = 1;
  }
};

/**
 * Replaces the uncaught-error behaviour, for example to turn `NotFound` into
 * a 404.
 * @param {(e: PolarError) => unknown} handler
 */
export function onUncaught(handler) {
  uncaught = handler;
}

/**
 * Hands an escaped Polar error to the current uncaught-error handler.
 * @param {PolarError} e
 * @returns {unknown}
 */
export function reportUncaught(e) {
  return uncaught(e);
}

/**
 * Called by generated code when no `match` arm matched.
 * Message: `no pattern matched (<file>:<line>:<column>)`.
 * @param {string} file
 * @param {number} line   1-based
 * @param {number} column 1-based, in code points
 * @returns {never}
 */
export function matchFailure(file, line, column) {
  throw new PolarMatchError(`no pattern matched (${file}:${line}:${column})`);
}

export const Log = {
  /** @param {string} s */
  info(s) {
    console.log(s);
  },

  /** @param {string} s */
  error(s) {
    console.error(s);
  },
};

export const String = {
  /**
   * Replaces **every** occurrence of `from`
   * @param {string} s
   * @param {string} from
   * @param {string} to
   * @returns {string}
   */
  replace(s, from, to) {
    return s.replaceAll(from, () => to);
  },

  /** @param {string} s @param {string} t @returns {string} */
  concat(s, t) {
    return s + t;
  },

  eq(s, t) {
    return s === t;
  },

  /** @param {string} s @param {string} part @returns {boolean} */
  contains(s, part) {
    return s.includes(part);
  },

  /** @param {string} s @param {string} suffix @returns {boolean} */
  ends_with(s, suffix) {
    return s.endsWith(suffix);
  },

  /** @param {string} s @returns {number} code points */
  length(s) {
    let n = 0;

    for (const _ of s) n++;

    return n;
  },

  /** @param {string} s @returns {string} */
  lowercase(s) {
    return s.toLowerCase();
  },

  /**
   * `s` repeated `n` times; `n <= 0` gives `""`.
   * @param {string} s @param {number} n @returns {string}
   */
  repeat(s, n) {
    return s.repeat(Math.max(0, Math.trunc(n)));
  },

  /**
   * Code points `[from, to)`, clamped to the string, like JavaScript's
   * `slice` (negative positions count from the end).
   * @param {string} s @param {number} from @param {number} to
   * @returns {string}
   */
  slice(s, from, to) {
    return Array.from(s).slice(from, to).join("");
  },

  /** @param {string} s @param {string} prefix @returns {boolean} */
  starts_with(s, prefix) {
    return s.startsWith(prefix);
  },

  /** @param {string} s @returns {string} */
  trim(s) {
    return s.trim();
  },

  /** @param {string} s @returns {string} */
  uppercase(s) {
    return s.toUpperCase();
  },

  /** @param {string} s @param {number} n @returns {number} */
  charCodeAt(s, n) {
    return s.charCodeAt(n);
  },

  /** @param {string} s @param {string} sep @returns {object} a `List<String>` */
  split(s, sep) {
    return list(s.split(sep));
  },

  split_once(s, sep) {
    const at = s.indexOf(sep);

    return at < 0
      ? none()
      : some({ before: s.slice(0, at), after: s.slice(at + sep.length) });
  },

  chars(s) {
    return list(Array.from(s));
  },
};

export const Int = {
  /** @param {number} n @returns {string} */
  to_string(n) {
    return JsString(n);
  },

  eq(a, b) {
    return a === b;
  },

  to_float(n) {
    return n;
  },

  to_base(n, radix) {
    return n.toString(radix);
  },

  /** Decimal digits with an optional leading `-`, within the safe range.
   * @param {string} s @returns {object} an `Option<Int>` */
  parse(s) {
    if (!/^-?[0-9]+$/.test(s)) {
      return none();
    }

    const n = Number(s);

    return Number.isSafeInteger(n) ? some(n) : none();
  },
};

export const Bool = {
  eq(a, b) {
    return a === b;
  },

  to_string(b) {
    return JsString(b);
  },
};

export const Float = {
  eq(a, b) {
    return a === b;
  },

  to_string(n) {
    return JsString(n);
  },

  to_int(n) {
    return Math.trunc(n);
  },

  /** @param {number} n @returns {number} */
  sqrt(n) {
    return Math.sqrt(n);
  },

  /** @param {number} n @returns {number} */
  abs(n) {
    return Math.abs(n);
  },

  /** @param {number} n @returns {number} */
  floor(n) {
    return n - (n % 1.0);
  },

  /** @param {number} n @returns {number} */
  sin(n) {
    return Math.sin(n);
  },

  /** @param {number} n @returns {number} */
  cos(n) {
    return Math.cos(n);
  },

  /** The angle of the point (`x`, `y`), in (-π, π].
   * @param {number} y @param {number} x @returns {number} */
  atan2(y, x) {
    return Math.atan2(y, x);
  },
};

export const Debug = {};

function toJsonValue(x) {
  if (x === null) return variant("JNull");
  if (typeof x === "boolean") return variant("JBool", x);
  if (typeof x === "number") return variant("JNumber", x);
  if (typeof x === "string") return variant("JString", x);
  if (Array.isArray(x)) return variant("JArray", list(x.map(toJsonValue)));

  const entries = Object.entries(x).map(([key, value]) => ({
    key,
    value: toJsonValue(value),
  }));

  return variant("JObject", list(entries));
}

function fromJsonValue(v) {
  switch (v.$) {
    case "JNull":
      return null;
    case "JArray":
      return [...each(payload(v))].map(fromJsonValue);
    case "JObject":
      return Object.fromEntries(
        [...each(payload(v))].map(({ key, value }) => [
          key,
          fromJsonValue(value),
        ]),
      );
    default:
      return payload(v);
  }
}

export class PolarRef {
  constructor(value) {
    this.value = value;
  }
}

export const RefRaw = {
  new(value) {
    return new PolarRef(value);
  },

  get(ref) {
    return ref.value;
  },

  set(ref, value) {
    ref.value = value;
  },
};

export const JsonRaw = {
  parse(text) {
    let raw;

    try {
      raw = JSON.parse(text);
    } catch (e) {
      return err(e.message);
    }

    return ok(toJsonValue(raw));
  },

  print(value) {
    return JSON.stringify(fromJsonValue(value));
  },

  lookup(fields, key) {
    for (const field of each(fields)) {
      if (field.key === key) {
        return some(field.value);
      }
    }

    return none();
  },
};

const bridgeConfig = { base: "/_polar", send: null };

class BridgeReject extends Error {
  constructor(status, detail) {
    super(detail);
    this.status = status;
  }
}

function encodeWire(codec, value) {
  return codec === null ? null : fromJsonValue(codec.to_json(value));
}

function decodeWire(codec, raw, path) {
  if (codec === null) {
    return {};
  }

  const result = codec.from_json(toJsonValue(raw ?? null), path);

  if (result.$ === "Err") {
    const { path: at, expected, found } = payload(result);
    const where = at === "" ? "" : `${at}: `;

    throw new BridgeReject(400, `${where}expected ${expected}, found ${found}`);
  }

  return payload(result);
}

function bridgeFailed(effect, op, status, detail) {
  return new PolarError("BridgeFailed", { effect, op, status, detail });
}

async function fetchSend(effect, op, body) {
  const url = `${bridgeConfig.base}/${encodeURIComponent(effect)}/${encodeURIComponent(op)}`;
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body,
  });

  return { status: response.status, body: await response.text() };
}

function readReply(reply, ret, throws) {
  let message;

  try {
    message = JSON.parse(reply.body);
  } catch {
    throw new BridgeReject(reply.status, reply.body);
  }

  if (reply.status !== 200 || message === null || typeof message !== "object") {
    throw new BridgeReject(reply.status, message?.error ?? reply.body);
  }

  if (Object.hasOwn(message, "throw")) {
    const { tag, value } = message.throw ?? {};
    const declared = throws.find(([t]) => t === tag);

    if (!declared) {
      throw new BridgeReject(reply.status, `undeclared error ${tag}`);
    }

    return { thrown: new PolarError(tag, decodeWire(declared[1], value, "")) };
  }

  return { value: decodeWire(ret, message.ok, "") };
}

async function bridgeCall(effect, op, params, ret, throws, args) {
  const body = JSON.stringify({
    args: params.map((codec, i) => encodeWire(codec, args[i])),
  });
  let reply;

  try {
    reply = await (bridgeConfig.send ?? fetchSend)(effect, op, body);
  } catch (e) {
    throw bridgeFailed(effect, op, 0, e?.message ?? JsString(e));
  }

  let read;

  try {
    read = readReply(reply, ret, throws);
  } catch (e) {
    if (e instanceof BridgeReject) {
      throw bridgeFailed(effect, op, e.status, e.message);
    }

    throw e;
  }

  if (read.thrown) {
    throw read.thrown;
  }

  return read.value;
}

function bridgeReply(status, message) {
  return { status, body: JSON.stringify(message) };
}

async function bridgeDispatch(tables, effect, op, body) {
  const table = Object.hasOwn(tables, effect) ? tables[effect] : null;
  const entry = table && Object.hasOwn(table, op) ? table[op] : null;

  if (!entry) {
    return bridgeReply(404, { error: `no bridged operation ${effect}.${op}` });
  }

  let args;

  try {
    const raw = JSON.parse(body)?.args;

    if (!Array.isArray(raw) || raw.length !== entry.params.length) {
      throw new BridgeReject(400, `expected ${entry.params.length} arguments`);
    }

    args = entry.params.map((codec, i) => decodeWire(codec, raw[i], `args[${i}]`));
  } catch (e) {
    const error = e instanceof BridgeReject ? e.message : "malformed request";

    return bridgeReply(400, { error });
  }

  try {
    const value = await entry.call(...args);

    return bridgeReply(200, { ok: encodeWire(entry.ret, value) });
  } catch (e) {
    const declared =
      isPolarError(e) && entry.throws.find(([tag]) => tag === e.type);

    if (declared) {
      const value = encodeWire(declared[1], e.value);

      return bridgeReply(200, { throw: { tag: e.type, value } });
    }

    console.error(e);

    return bridgeReply(500, { error: "internal error" });
  }
}

function bridgeHandler(tables, { base = "/_polar" } = {}) {
  const prefix = `${base.replace(/\/+$/, "")}/`;
  const json = { "content-type": "application/json" };

  return async (request) => {
    const url = new URL(request.url);

    if (!url.pathname.startsWith(prefix)) {
      return null;
    }

    const [effect, op, ...rest] = url.pathname.slice(prefix.length).split("/");

    if (!effect || !op || rest.length > 0) {
      const out = bridgeReply(404, { error: "not a bridged operation" });

      return new Response(out.body, { status: out.status, headers: json });
    }

    if (request.method !== "POST") {
      const out = bridgeReply(405, { error: "bridged operations take POST" });

      return new Response(out.body, {
        status: out.status,
        headers: { ...json, allow: "POST" },
      });
    }

    const out = await bridgeDispatch(
      tables,
      decodeURIComponent(effect),
      decodeURIComponent(op),
      await request.text(),
    );

    return new Response(out.body, { status: out.status, headers: json });
  };
}

function nodeListener(handle, { limit = 1_000_000 } = {}) {
  return async (req, res) => {
    const chunks = [];
    let size = 0;

    for await (const chunk of req) {
      size += chunk.length;

      if (size > limit) {
        res.writeHead(413, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: "request too large" }));
        return;
      }

      chunks.push(chunk);
    }

    const bodyless = req.method === "GET" || req.method === "HEAD";
    const request = new Request(new URL(req.url, "http://localhost"), {
      method: req.method,
      headers: Object.entries(req.headers).filter(([, v]) => typeof v === "string"),
      body: bodyless ? undefined : Buffer.concat(chunks),
    });
    const response = await handle(request);

    if (!response) {
      res.writeHead(404, { "content-type": "application/json" });
      res.end(JSON.stringify({ error: "not found" }));
      return;
    }

    res.writeHead(response.status, Object.fromEntries(response.headers));
    res.end(Buffer.from(await response.arrayBuffer()));
  };
}

function bridgeNodeListener(tables, { limit, ...options } = {}) {
  return nodeListener(bridgeHandler(tables, options), { limit });
}

export const bridge = {
  configure({ base, send } = {}) {
    if (base !== undefined) {
      bridgeConfig.base = base.replace(/\/+$/, "");
    }

    if (send !== undefined) {
      bridgeConfig.send = send;
    }
  },

  op(effect, op, params, ret, throws) {
    return (...args) =>
      bridgeCall(effect, op, params, ret, throws, args.slice(0, params.length));
  },

  table(effect, bound, ops) {
    return Object.fromEntries(
      Object.entries(ops).map(([op, [params, ret, throws]]) => [
        op,
        { params, ret, throws, call: (...args) => bound[op](...args) },
      ]),
    );
  },

  dispatch: bridgeDispatch,
  handler: bridgeHandler,
  nodeListener: bridgeNodeListener,
};

/**
 * A fetch `Request` as the Polar `Std.Http.Request` record.
 * @param {Request} request
 * @returns {Promise<object>}
 */
async function toPolarRequest(request) {
  const url = new URL(request.url);
  const headers = [...request.headers].map(([name, value]) => ({
    name: name.toLowerCase(),
    value,
  }));

  return {
    method: request.method,
    path: url.pathname,
    query: url.search.replace(/^\?/, ""),
    headers: list(headers),
    body: await request.text(),
  };
}

/**
 * A Polar `Std.Http.Response` record as a fetch `Response`.
 * @param {object} response
 * @returns {Response}
 */
function fromPolarResponse(response) {
  const headers = [...each(response.headers)].map((h) => [h.name, h.value]);
  const empty = response.status === 204 || response.status === 304;

  return new Response(empty ? null : response.body, {
    status: response.status,
    headers,
  });
}

function httpHandler(router) {
  return async (request) => {
    let routed;

    try {
      routed = await router(await toPolarRequest(request));
    } catch (e) {
      console.error(e);

      return new Response(JSON.stringify({ error: "internal error" }), {
        status: 500,
        headers: { "content-type": "application/json" },
      });
    }

    return routed.$ === "Some" ? fromPolarResponse(payload(routed)) : null;
  };
}

const fileTypes = {
  ".css": "text/css; charset=utf-8",
  ".html": "text/html; charset=utf-8",
  ".ico": "image/x-icon",
  ".jpg": "image/jpeg",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json",
  ".map": "application/json",
  ".mjs": "text/javascript; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".txt": "text/plain; charset=utf-8",
  ".woff2": "font/woff2",
};

function httpFiles(root, { prefix = "/" } = {}) {
  const base = prefix.endsWith("/") ? prefix : `${prefix}/`;

  return async (request) => {
    if (request.method !== "GET" && request.method !== "HEAD") {
      return null;
    }

    const url = new URL(request.url);

    if (!url.pathname.startsWith(base)) {
      return null;
    }

    const { readFile, stat } = await import("node:fs/promises");
    const { extname, resolve, sep } = await import("node:path");
    const top = resolve(root);
    let file;

    try {
      file = resolve(top, decodeURIComponent(url.pathname.slice(base.length)));
    } catch {
      return null;
    }

    if (!file.startsWith(top + sep)) {
      return null;
    }

    try {
      if (!(await stat(file)).isFile()) {
        return null;
      }

      const body = request.method === "HEAD" ? null : await readFile(file);
      const type = fileTypes[extname(file)] ?? "application/octet-stream";

      return new Response(body, { status: 200, headers: { "content-type": type } });
    } catch {
      return null;
    }
  };
}

function httpCompose(...handlers) {
  return async (request) => {
    for (const handle of handlers) {
      const response = await handle(request.clone());

      if (response) {
        return response;
      }
    }

    return null;
  };
}

/**
 * Serving a Polar `router` over HTTP. Every handler has the fetch shape
 * `(Request) => Promise<Response | null>`, where `null` means "not mine":
 * `handler(router)` runs a generated `routes` router, `compose(...)` tries
 * handlers in order, and `nodeListener(handler)` serves one with `node:http`,
 * answering 404 when it returns `null`. An error escaping the router is logged
 * and answered with a 500 that carries no details. `files(dir, { prefix })`
 * serves the files under `dir` at `prefix`, on Node only.
 */
export const http = {
  handler: httpHandler,
  compose: httpCompose,
  files: httpFiles,
  nodeListener,
  toRequest: toPolarRequest,
  fromResponse: fromPolarResponse,
};
