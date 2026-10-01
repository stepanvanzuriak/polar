import "../shared/dom_stub.mjs";
import { createServer } from "node:http";

const dist = new URL(`${process.argv[2] ?? "dist"}/`, `file://${process.cwd()}/`);
const load = (path) => import(new URL(path, dist).href);

const { bridges } = await load("Node/_polar/bridges.js");
const server = await load("Node/_polar/runtime.js");
const http = createServer(server.bridge.nodeListener(bridges));

await new Promise((resolve) => http.listen(0, "127.0.0.1", resolve));

try {
  const { port } = http.address();
  const client = await load("Browser/_polar/runtime.js");

  client.bridge.configure({ base: `http://127.0.0.1:${port}/_polar` });

  const app = await load("Browser/main.js");

  await app.start();
} finally {
  http.close();
}
