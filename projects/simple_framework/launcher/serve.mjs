import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

export async function serve(manifest) {
  const options = manifest.options ?? {};
  const serverHost = options.server ?? "Node";
  const clientHost = options.client ?? "Browser";
  const server = manifest.hosts?.[serverHost] ?? manifest.out;
  const client = manifest.hosts?.[clientHost];
  const main = manifest.main ?? "main.js";
  const load = (dir, file) => import(pathToFileURL(join(dir, file)).href);
  const rt = await load(server, "_polar/runtime.js");
  const program = await load(server, main);

  if (typeof program.router !== "function") {
    throw new Error(`\`${main}\` for ${serverHost} doesn't export \`router\``);
  }

  const handlers = [];

  if (client) {
    handlers.push(rt.http.files(client, { prefix: "/_client/" }));
  }

  if (existsSync(join(server, "_polar/bridges.js"))) {
    const { bridges } = await load(server, "_polar/bridges.js");

    handlers.push(rt.bridge.handler(bridges));
  }

  handlers.push(rt.http.handler(program.router));

  const listener = createServer(rt.http.nodeListener(rt.http.compose(...handlers)));
  const hostname = options.hostname ?? "127.0.0.1";

  await new Promise((resolve, reject) => {
    listener.once("error", reject);
    listener.listen(options.port ?? 3000, hostname, resolve);
  });

  return listener;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const manifest = JSON.parse(await readFile(process.argv[2], "utf8"));
  const listener = await serve(manifest);
  const { address, port } = listener.address();

  console.log(`${manifest.project} is listening on http://${address}:${port}`);

  const stop = () => {
    listener.closeAllConnections();
    listener.close();
  };

  process.once("SIGINT", stop);
  process.once("SIGTERM", stop);
}
