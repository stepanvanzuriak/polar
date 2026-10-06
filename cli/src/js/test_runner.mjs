import { describe, test } from "node:test";
import { readdirSync } from "node:fs";
import { join, sep } from "node:path";
import { pathToFileURL } from "node:url";

const [out] = process.argv.slice(2);
const files = readdirSync(out, { recursive: true })
  .filter((file) => file.endsWith("_test.js"))
  .filter((file) => !file.split(sep).some((part) => part.startsWith("_")))
  .sort();

if (files.length === 0) {
  console.log("no tests found (looked for *_test.px under src)");
}

for (const file of files) {
  const module = await import(pathToFileURL(join(out, file)).href);
  const names = Object.keys(module).filter((name) => name.startsWith("test_")).sort();

  describe(file.replace(/\.js$/, ".px"), () => {
    for (const name of names) {
      test(name, async () => {
        try {
          await module[name]();
        } catch (error) {
          throw readable(error);
        }
      });
    }
  });
}

function readable(error) {
  if (error?.type !== "Assert.Failed") return error;

  const message = error.value._0;
  const failure = new Error(message);

  failure.name = "AssertionError";
  failure.stack = `AssertionError: ${message}\n${error.stack.split("\n").slice(1).join("\n")}`;

  return failure;
}
