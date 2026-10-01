import { test } from "node:test";
import assert from "node:assert/strict";
import * as rt from "./runtime.js";

test("list_builds_cons_cells", () => {
  const nil = { $: "Nil" };

  assert.deepEqual(rt.list([]), nil);
  assert.deepEqual(rt.list([1, 2]), {
    $: "Cons",
    _0: 1,
    _1: { $: "Cons", _0: 2, _1: nil },
  });
  assert.deepEqual(rt.list([1], rt.list([2])), rt.list([1, 2]));
});

test("match_failure_throws", () => {
  assert.throws(
    () => rt.matchFailure("f.px", 12, 3),
    (err) => {
      assert.ok(err instanceof rt.PolarMatchError);
      assert.equal(err.message, "no pattern matched (f.px:12:3)");
      return true;
    },
  );
});

test("string_replace_is_global", () => {
  assert.equal(rt.String.replace("a b c", " ", "-"), "a-b-c");
});

test("log_targets", (t) => {
  const log = t.mock.method(console, "log", () => {});
  const error = t.mock.method(console, "error", () => {});

  rt.Log.info("to stdout");
  rt.Log.error("to stderr");

  assert.deepEqual(
    log.mock.calls.map((c) => c.arguments),
    [["to stdout"]],
  );
  assert.deepEqual(
    error.mock.calls.map((c) => c.arguments),
    [["to stderr"]],
  );
});

test("show_renders_records_and_variants", () => {
  const record = rt.show({ a: 1 });
  const variant = rt.show({ $: "A", _0: 1 });

  assert.equal(typeof record, "string");
  assert.equal(typeof variant, "string");

  assert.match(record, /\ba\b/);
  assert.match(record, /\b1\b/);
  assert.match(variant, /\bA\b/);
  assert.match(variant, /\b1\b/);

  assert.doesNotMatch(variant, /\$|_0/);

  assert.notEqual(record, variant);
  assert.notEqual(rt.show({ $: "A" }), rt.show({ A: {} }));
});

test("string_basics", () => {
  assert.equal(rt.String.concat("ab", "cd"), "abcd");
  assert.equal(rt.String.contains("hello", "ell"), true);
  assert.equal(rt.String.starts_with("hello", "he"), true);
  assert.equal(rt.String.ends_with("hello", "lo"), true);
  assert.equal(rt.String.lowercase("HeLLo"), "hello");
  assert.equal(rt.String.uppercase("héllo"), "HÉLLO");
  assert.equal(rt.String.trim("  a b \n"), "a b");
  assert.equal(rt.String.repeat("ab", 3), "ababab");
  assert.equal(rt.String.repeat("ab", -1), "");
});

test("float_math", () => {
  assert.equal(rt.Float.floor(2.7), 2);
  assert.equal(rt.Float.sin(0), 0);
  assert.equal(rt.Float.cos(0), 1);
  assert.equal(rt.Float.atan2(1, 0), Math.PI / 2);
  assert.equal(rt.Float.atan2(0, -1), Math.PI);
});

test("string_positions_count_code_points", () => {
  assert.equal(rt.String.length("😀a"), 2);
  assert.equal(rt.String.slice("a😀bc", 1, 3), "😀b");
  assert.equal(rt.String.slice("abc", 1, 99), "bc");
});

test("int_to_string", () => {
  assert.equal(rt.Int.to_string(42), "42");
  assert.equal(rt.Int.to_string(-7), "-7");
});

test("raise_throws_a_polar_error", () => {
  assert.throws(
    () => rt.raise("App.Missing", { $: "Missing", _0: "k" }),
    (err) => {
      assert.ok(err instanceof rt.PolarError);
      assert.equal(err.type, "App.Missing");
      assert.deepEqual(err.value, { $: "Missing", _0: "k" });
      assert.equal(err.message, 'uncaught error: Missing("k")');
      return true;
    },
  );
});

test("handles_only_listed_polar_errors", () => {
  const missing = new rt.PolarError("App.Missing", { $: "Missing", _0: "k" });
  const branded = {
    [Symbol.for("polar.error")]: true,
    type: "App.Missing",
    value: { $: "Missing", _0: "k" },
  };

  assert.equal(rt.handles(missing, ["App.Missing"]), true);
  assert.equal(rt.handles(branded, ["App.Missing"]), true);
  assert.equal(rt.handles(missing, ["App.Invalid"]), false);
  assert.equal(rt.handles(new TypeError("x"), ["App.Missing"]), false);
  assert.equal(rt.handles(undefined, ["App.Missing"]), false);
});

test("runtime_on_uncaught", (t) => {
  const error = t.mock.method(console, "error", () => {});
  const seen = [];
  const failure = new rt.PolarError("App.Invalid", { $: "Empty" });

  rt.onUncaught((e) => seen.push(e));
  rt.reportUncaught(failure);

  assert.deepEqual(seen, [failure]);
  assert.equal(error.mock.callCount(), 0);
});

test("extern_boundaries_convert_results", async () => {
  const echo = async (value) => value;
  const option = ["option", null];
  const strings = ["list", null];

  assert.deepEqual(await rt.extern(() => undefined, [], "unit")(), {});
  assert.deepEqual(await rt.extern(echo, [null], option)(null), { $: "None" });
  assert.deepEqual(await rt.extern(echo, [null], option)(undefined), {
    $: "None",
  });
  assert.deepEqual(await rt.extern(echo, [null], option)(""), {
    $: "Some",
    _0: "",
  });
  assert.deepEqual(
    await rt.extern(echo, [null], strings)(["a", "b"]),
    rt.list(["a", "b"]),
  );
  assert.deepEqual(
    await rt.extern(echo, [null], strings)(new Set([1])),
    rt.list([1]),
  );
});

test("extern_boundaries_are_sync_for_sync_javascript", () => {
  const id = (value) => value;

  assert.deepEqual(rt.extern(id, [null], ["option", null])(3), {
    $: "Some",
    _0: 3,
  });
  assert.deepEqual(rt.extern(() => undefined, [], "unit")(), {});
});

test("extern_boundaries_convert_arguments", () => {
  const id = (value) => value;
  const some = { $: "Some", _0: rt.list([1, 2]) };

  assert.deepEqual(rt.extern(id, [["list", null]], null)(rt.list([1, 2])), [
    1, 2,
  ]);
  assert.equal(rt.extern(id, [["option", null]], null)({ $: "None" }), null);
  assert.deepEqual(
    rt.extern(id, [["option", ["list", null]]], null)(some),
    [1, 2],
  );
  assert.deepEqual(
    rt.extern(id, [{ tags: ["list", null] }], null)({
      title: "t",
      tags: rt.list(["a"]),
    }),
    { title: "t", tags: ["a"] },
  );
  assert.deepEqual(
    rt.extern(id, [null], { tags: ["list", null] })({ title: "t", tags: ["a"] }),
    { title: "t", tags: rt.list(["a"]) },
  );
});

test("extern_ignores_the_async_flag", () => {
  const count = (...args) => args.length;

  assert.equal(rt.extern(count, [null], null)(1, rt.ASYNC), 1);
});

test("poly_picks_the_version_the_caller_asks_for", async () => {
  const f = rt.poly(
    1,
    (x) => x + 1,
    async (x) => x + 2,
  );

  assert.equal(f(1), 2);
  assert.equal(await f(1, rt.ASYNC), 3);
  assert.equal(f(1, "something else"), 2);
});

test("async_is_shared_across_runtime_copies", () => {
  assert.equal(rt.ASYNC, Symbol.for("polar.async"));
});

const intCodec = {
  to_json: (n) => ({ $: "JNumber", _0: n }),
  from_json: (v, path) =>
    v.$ === "JNumber"
      ? { $: "Ok", _0: v._0 }
      : { $: "Err", _0: { path, expected: "Int", found: v.$ } },
};

const missingCodec = {
  to_json: (m) => ({ $: "JNumber", _0: m._0 }),
  from_json: (v) => ({ $: "Ok", _0: { $: "Missing", _0: v._0 } }),
};

function server(ops) {
  const bound = {
    double: async (n) => n * 2,
    fail: async (n) => rt.raise("App.Missing", { $: "Missing", _0: n }),
    crash: async () => {
      throw new Error("boom");
    },
    touch: async () => ({}),
    ...ops,
  };

  return {
    Math: rt.bridge.table("Math", bound, {
      double: [[intCodec], intCodec, []],
      fail: [[intCodec], intCodec, [["App.Missing", missingCodec]]],
      crash: [[], intCodec, []],
      touch: [[intCodec], null, []],
    }),
  };
}

function connect(tables) {
  rt.bridge.configure({
    send: (effect, op, body) => rt.bridge.dispatch(tables, effect, op, body),
  });
}

test("bridge_round_trips_a_value", async () => {
  connect(server());

  const double = rt.bridge.op("Math", "double", [intCodec], intCodec, []);

  assert.equal(await double(21, rt.ASYNC), 42);
});

test("bridge_unit_crosses_as_null", async () => {
  const tables = server();
  let sent;

  rt.bridge.configure({
    send: async (effect, op, body) => {
      const reply = await rt.bridge.dispatch(tables, effect, op, body);

      sent = reply.body;
      return reply;
    },
  });

  const touch = rt.bridge.op("Math", "touch", [intCodec], null, []);

  assert.deepEqual(await touch(1), {});
  assert.equal(sent, '{"ok":null}');
});

test("bridge_declared_throw_is_raised_on_the_client", async () => {
  connect(server());

  const fail = rt.bridge.op("Math", "fail", [intCodec], intCodec, [
    ["App.Missing", missingCodec],
  ]);

  await assert.rejects(fail(7), (e) => {
    assert.ok(rt.handles(e, ["App.Missing"]));
    assert.deepEqual(e.value, { $: "Missing", _0: 7 });
    return true;
  });
});

test("bridge_server_errors_stay_on_the_server", async (t) => {
  const error = t.mock.method(console, "error", () => {});

  connect(server());

  const crash = rt.bridge.op("Math", "crash", [], intCodec, []);

  await assert.rejects(crash(), (e) => {
    assert.equal(e.type, "BridgeFailed");
    assert.equal(e.value.status, 500);
    assert.equal(e.value.detail, "internal error");
    return true;
  });
  assert.equal(error.mock.callCount(), 1);
});

test("bridge_rejects_malformed_arguments", async () => {
  const tables = server();
  let called = false;
  const guarded = server({
    double: async () => {
      called = true;
    },
  });

  assert.deepEqual(
    await rt.bridge.dispatch(tables, "Math", "double", '{"args":["x"]}'),
    { status: 400, body: '{"error":"args[0]: expected Int, found JString"}' },
  );
  assert.equal(
    (await rt.bridge.dispatch(guarded, "Math", "double", '{"args":[]}')).status,
    400,
  );
  assert.equal(
    (await rt.bridge.dispatch(guarded, "Math", "double", "not json")).status,
    400,
  );
  assert.equal(called, false);
});

test("bridge_unknown_operations_are_404", async () => {
  const tables = server();

  for (const [effect, op] of [
    ["Math", "nope"],
    ["Nope", "double"],
    ["Math", "__proto__"],
    ["constructor", "double"],
  ]) {
    const reply = await rt.bridge.dispatch(tables, effect, op, '{"args":[]}');

    assert.equal(reply.status, 404, `${effect}.${op}`);
  }
});

test("bridge_network_failure_is_bridge_failed", async () => {
  rt.bridge.configure({
    send: async () => {
      throw new Error("offline");
    },
  });

  const double = rt.bridge.op("Math", "double", [intCodec], intCodec, []);

  await assert.rejects(double(1), (e) => {
    assert.equal(e.type, "BridgeFailed");
    assert.deepEqual(e.value, {
      effect: "Math",
      op: "double",
      status: 0,
      detail: "offline",
    });
    return true;
  });
});

test("bridge_handler_routes_under_its_base", async () => {
  const handle = rt.bridge.handler(server(), { base: "/api/" });
  const post = (path, body) =>
    handle(new Request(`http://x${path}`, { method: "POST", body }));

  assert.equal(await post("/other/Math/double", "{}"), null);

  const ok = await post("/api/Math/double", '{"args":[4]}');

  assert.equal(ok.status, 200);
  assert.deepEqual(await ok.json(), { ok: 8 });
  assert.equal((await post("/api/Math", "{}")).status, 404);

  const get = await handle(new Request("http://x/api/Math/double"));

  assert.equal(get.status, 405);
  assert.equal(get.headers.get("allow"), "POST");
});

test("bridge_node_listener_serves_http", async () => {
  const { createServer } = await import("node:http");
  const http = createServer(rt.bridge.nodeListener(server(), { limit: 64 }));

  await new Promise((resolve) => http.listen(0, "127.0.0.1", resolve));

  try {
    const base = `http://127.0.0.1:${http.address().port}/_polar`;

    rt.bridge.configure({ base, send: null });

    const double = rt.bridge.op("Math", "double", [intCodec], intCodec, []);

    assert.equal(await double(5), 10);

    const big = await fetch(`${base}/Math/double`, {
      method: "POST",
      body: JSON.stringify({ args: [1], pad: "x".repeat(100) }),
    });

    assert.equal(big.status, 413);
  } finally {
    http.close();
    rt.bridge.configure({ base: "/_polar" });
  }
});

test("int_parse_is_strict_decimal", () => {
  assert.deepEqual(rt.Int.parse("42"), { $: "Some", _0: 42 });
  assert.deepEqual(rt.Int.parse("-7"), { $: "Some", _0: -7 });

  for (const bad of ["4x", "", "+1", " 1", "1e3", "9007199254740993"]) {
    assert.deepEqual(rt.Int.parse(bad), { $: "None" }, bad);
  }
});

test("string_split_keeps_empty_parts", () => {
  assert.deepEqual(rt.String.split("/a/b", "/"), rt.list(["", "a", "b"]));
});

function fakeRouter(answer) {
  return async (request) => answer(request);
}

const ok = (status, body) => ({
  $: "Some",
  _0: {
    status,
    headers: rt.list([{ name: "content-type", value: "text/plain" }]),
    body,
  },
});

test("http_handler_converts_both_ways", async () => {
  let seen;
  const handle = rt.http.handler(
    fakeRouter((request) => {
      seen = request;
      return ok(201, `got ${request.body}`);
    }),
  );
  const response = await handle(
    new Request("http://x/posts?draft=1", {
      method: "POST",
      headers: { "X-Thing": "yes" },
      body: "hi",
    }),
  );

  assert.equal(seen.method, "POST");
  assert.equal(seen.path, "/posts");
  assert.equal(seen.query, "draft=1");
  assert.ok(
    [...walk(seen.headers)].some(
      (h) => h.name === "x-thing" && h.value === "yes",
    ),
  );
  assert.equal(response.status, 201);
  assert.equal(response.headers.get("content-type"), "text/plain");
  assert.equal(await response.text(), "got hi");
});

function* walk(list) {
  for (let rest = list; rest.$ === "Cons"; rest = rest._1) {
    yield rest._0;
  }
}

test("http_handler_none_is_not_mine", async () => {
  const handle = rt.http.handler(fakeRouter(() => ({ $: "None" })));

  assert.equal(await handle(new Request("http://x/")), null);
});

test("http_handler_answers_500_when_the_router_throws", async (t) => {
  t.mock.method(console, "error", () => {});

  const handle = rt.http.handler(
    fakeRouter(() => rt.raise("App.Boom", { $: "Boom" })),
  );
  const response = await handle(new Request("http://x/"));

  assert.equal(response.status, 500);
  assert.ok(!(await response.text()).includes("Boom"));
});

test("http_compose_tries_handlers_in_order_with_bridges", async () => {
  const routes = rt.http.handler(fakeRouter(() => ({ $: "None" })));
  const handle = rt.http.compose(routes, rt.bridge.handler(server()));
  const response = await handle(
    new Request("http://x/_polar/Math/double", {
      method: "POST",
      body: JSON.stringify({ args: [21] }),
    }),
  );

  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { ok: 42 });
  assert.equal(await handle(new Request("http://x/elsewhere")), null);
});

test("http_node_listener_serves_and_404s", async () => {
  const { createServer } = await import("node:http");
  const handle = rt.http.handler(
    fakeRouter((request) =>
      request.path === "/hello" ? ok(200, "hello") : { $: "None" },
    ),
  );
  const server = createServer(rt.http.nodeListener(handle));

  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));

  try {
    const base = `http://127.0.0.1:${server.address().port}`;
    const hello = await fetch(`${base}/hello`);

    assert.equal(hello.status, 200);
    assert.equal(await hello.text(), "hello");
    assert.equal((await fetch(`${base}/missing`)).status, 404);
  } finally {
    server.close();
  }
});

test("http_files_serve_under_a_prefix_and_stay_inside", async () => {
  const { mkdtemp, mkdir, writeFile } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const dir = await mkdtemp(join(tmpdir(), "polar-files-"));
  const bytes = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0xff, 0x00]);

  await mkdir(join(dir, "public/nested"), { recursive: true });
  await writeFile(join(dir, "public/nested/app.js"), "export const x = 1;\n");
  await writeFile(join(dir, "public/logo.png"), bytes);
  await writeFile(join(dir, "secret.txt"), "no");

  const files = rt.http.files(join(dir, "public"), { prefix: "/static" });
  const get = (path, method = "GET") =>
    files(new Request(`http://localhost${path}`, { method }));

  const js = await get("/static/nested/app.js");

  assert.equal(js.status, 200);
  assert.equal(js.headers.get("content-type"), "text/javascript; charset=utf-8");
  assert.equal(await js.text(), "export const x = 1;\n");
  assert.equal(await get("/other/nested/app.js"), null);
  assert.equal(await get("/static/missing.js"), null);
  assert.equal(await get("/static/nested"), null);
  assert.equal(await get("/static/..%2fsecret.txt"), null);
  assert.equal(await get("/static/%E0%A4%A"), null);
  assert.equal(await get("/static/nested/app.js", "POST"), null);

  const { createServer } = await import("node:http");
  const server = createServer(rt.http.nodeListener(files));

  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));

  try {
    const png = await fetch(`http://127.0.0.1:${server.address().port}/static/logo.png`);

    assert.equal(png.headers.get("content-type"), "image/png");
    assert.deepEqual(Buffer.from(await png.arrayBuffer()), bytes);
  } finally {
    server.close();
  }
});
