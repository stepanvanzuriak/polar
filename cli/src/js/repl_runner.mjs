import { createInterface } from "node:readline";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const [dir] = process.argv.slice(2);
const MARK = "\u0001polar-repl ";

function reply(message) {
  process.stdout.write(`${MARK}${JSON.stringify(message)}\n`);
}

function describe(error) {
  if (error?.[Symbol.for("polar.error")]) {
    return error.message;
  }

  return `${error?.name ?? "Error"}: ${error?.message ?? String(error)}`;
}

for await (const line of createInterface({ input: process.stdin })) {
  const request = JSON.parse(line);

  try {
    const module = await import(pathToFileURL(join(dir, request.file)).href);
    const value = await module[request.call]();

    reply({ ok: true, value: typeof value === "string" ? value : null });
  } catch (error) {
    reply({ ok: false, error: describe(error) });
  }
}
