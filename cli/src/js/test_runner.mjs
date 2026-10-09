import { test } from "node:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const [out] = process.argv.slice(2);
const { tests } = JSON.parse(readFileSync(join(out, "_polar", "tests.json"), "utf8"));

if (tests.length === 0) {
  console.log("no tests found (looked for *_test.px under src)");
}

const modules = new Map();

for (const { js } of tests) {
  if (!modules.has(js)) {
    modules.set(js, await import(pathToFileURL(join(out, js)).href));
  }
}

for (const { js, name } of tests) {
  const module = modules.get(js);

  test(qualified(js, name), async () => {
    try {
      await module[name]();
    } catch (error) {
      throw readable(error);
    }
  });
}

function qualified(js, name) {
  return [...js.replace(/\.js$/, "").split("/"), name].join("::");
}

function readable(error) {
  if (error?.type !== "Assert.Failed") return error;

  const message = error.value._0;
  const failure = new Error(message);

  failure.name = "AssertionError";
  failure.stack = `AssertionError: ${message}\n${error.stack.split("\n").slice(1).join("\n")}`;

  return failure;
}
