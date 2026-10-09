import { ASYNC, isPolarError, payload, show, variant } from "../../runtime.js";
import { relative } from "node:path";
import { fileURLToPath } from "node:url";

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
