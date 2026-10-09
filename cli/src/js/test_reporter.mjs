import { readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const color = process.stdout.isTTY && !process.env.NO_COLOR;
const paint = (code, text) => (color ? `\x1b[${code}m${text}\x1b[0m` : text);
const green = (text) => paint(32, text);
const red = (text) => paint(31, text);

export default async function* report(source) {
  const start = performance.now();
  const failures = [];
  let queued = 0;
  let passed = 0;
  let announced = false;

  for await (const { type, data } of source) {
    if (data?.nesting !== 0) continue;

    if (type === "test:enqueue") {
      queued += 1;
      continue;
    }

    if (!announced && ["test:pass", "test:fail", "test:plan"].includes(type)) {
      announced = true;
      yield `\nrunning ${queued} ${queued === 1 ? "test" : "tests"}\n`;
    }

    if (type === "test:pass") {
      passed += 1;
      yield `test ${data.name} ... ${green("ok")}\n`;
    } else if (type === "test:fail") {
      failures.push(data);
      yield `test ${data.name} ... ${red("FAILED")}\n`;
    }
  }

  if (failures.length > 0) {
    yield "\nfailures:\n";

    for (const failure of failures) {
      yield `\n---- ${failure.name} ----\n${describe(failure.details.error)}\n`;
    }

    yield `\nfailures:\n${failures.map((f) => `    ${f.name}\n`).join("")}`;
  }

  const status = failures.length === 0 ? green("ok") : red("FAILED");
  const seconds = ((performance.now() - start) / 1000).toFixed(2);

  yield `\ntest result: ${status}. ${passed} passed; ${failures.length} failed; ` +
    `${total() - queued} filtered out; finished in ${seconds}s\n\n`;
}

function describe(error) {
  const cause = error?.cause ?? error;
  const message = cause?.message ?? String(cause);
  const frames = String(cause?.stack ?? "")
    .split("\n")
    .slice(1)
    .map((line) => line.trim())
    .filter((line) => line.startsWith("at "))
    .filter((line) => !/node:|test_runner\.mjs|\/_polar\//.test(line));
  const location = frames.map(source).find(Boolean);

  if (location) return `failed at ${location}:\n${message}`;

  return [message, ...frames.map((frame) => `    ${frame}`)].join("\n");
}

function source(frame) {
  const found = frame.match(/((?:file:\/\/)?[^\s()]+\.px):(\d+):(\d+)\)?$/);

  if (!found) return null;

  const [, file, line, column] = found;
  const path = file.startsWith("file://") ? fileURLToPath(file) : file;

  return `${relative(process.cwd(), path)}:${line}:${column}`;
}

function total() {
  try {
    const out = process.argv[2];

    return JSON.parse(readFileSync(join(out, "_polar", "tests.json"), "utf8")).tests.length;
  } catch {
    return 0;
  }
}
