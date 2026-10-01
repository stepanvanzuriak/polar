import "../shared/dom_stub.mjs";
import { fileURLToPath } from "node:url";
import { serve } from "../simple_framework/launcher/serve.mjs";

const dist = new URL(`${process.argv[2] ?? "dist"}/`, `file://${process.cwd()}/`);
const load = (path) => import(new URL(path, dist).href);
const server = await serve({
  version: 1,
  project: "web_posts",
  main: "main.js",
  hosts: {
    Node: fileURLToPath(new URL("Node", dist)),
    Browser: fileURLToPath(new URL("Browser", dist)),
  },
  options: { port: 0 },
});

const base = `http://127.0.0.1:${server.address().port}`;

async function call(method, path, body) {
  const response = await fetch(`${base}${path}`, {
    method,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();

  console.log(`${method} ${path} -> ${response.status} ${text}`.trimEnd());
}

async function fetched(path) {
  const response = await fetch(`${base}${path}`);
  const text = await response.text();
  const start = text.includes("export async function start(") ? " with start" : "";

  console.log(`GET ${path} -> ${response.status} ${response.headers.get("content-type")}${start}`);
}

try {
  await call("GET", "/posts");
  await call("POST", "/posts", { title: "First", author_id: 1 });
  await call("POST", "/posts", { title: "Second", author_id: 2 });
  await call("GET", "/posts/1");
  await call("GET", "/posts/99");
  await call("GET", "/posts/abc");
  await call("POST", "/posts", { title: 3, author_id: 1 });
  await call("DELETE", "/posts/1");
  await call("DELETE", "/posts/1");
  await call("GET", "/posts");
  await call("POST", "/posts", { title: `<script>alert("hi")</script> & co`, author_id: 1 });
  await call("GET", "/");
  await fetched("/_client/main.js");
  await fetched("/_client/..%2fNode%2fmain.js");

  const client = await load("Browser/_polar/runtime.js");

  client.bridge.configure({ base: `${base}/_polar` });

  const app = await load("Browser/main.js");

  await app.start();
} finally {
  server.close();
}
