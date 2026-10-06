// Runs a compiled Polar program for `polar run`:
//
//   node --enable-source-maps launcher.mjs <main.js> <polar-file> [runtime.js]
//
// It imports the module, awaits its exported `main`, and on a throw prints
// the stack filtered down to frames in `.px` files. The filtering lives
// here because only the JavaScript side sees the source-mapped stack.

import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

const own = process.argv.slice(2);
const dashes = own.indexOf("--");
const [mainPath, polarFile, runtimePath] = dashes < 0 ? own : own.slice(0, dashes);
const program = await import(pathToFileURL(mainPath).href);

if (typeof program.main !== "function") {
  process.stderr.write(
    `error: \`${polarFile}\` does not export \`main\`\n` +
      "  = help: add `main` to the `exports` zone\n",
  );
  process.exit(1);
}

try {
  await program.main();
  process.exitCode ??= 0;
} catch (error) {
  if (error?.[Symbol.for("polar.error")]) {
    const runtime = await import(
      pathToFileURL(runtimePath ?? join(dirname(mainPath), "_polar", "runtime.js"))
        .href
    );

    await runtime.reportUncaught(error);
  } else {
    report(error);
  }
}

function report(error) {
  const name = error?.name ?? "Error";
  const message = error?.message ?? String(error);
  const stack = typeof error?.stack === "string" ? error.stack : "";
  const frames = stack.split("\n").filter((line) => /^\s+at /.test(line));

  // Frames that resolve to Polar source. Everything else — the runtime,
  // this launcher, Node's internals, and generated code with no mapping
  // (the compiler's async scaffolding) — is dropped.
  const polar = frames.filter(
    (line) =>
      /\.px:\d+:\d+\)?$/.test(line) &&
      !line.includes("node:internal") &&
      !line.includes("runtime.js") &&
      !line.includes("launcher.mjs"),
  );

  process.stderr.write(`error: uncaught ${name}: ${message}\n`);

  // A bad trace beats no trace.
  const shown = polar.length > 0 ? polar : frames;

  if (shown.length > 0) {
    process.stderr.write(shown.join("\n") + "\n");
  }

  process.exitCode = 1;
}
