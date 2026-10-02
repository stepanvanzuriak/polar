import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const config = JSON.parse(
  readFileSync(join(here, "_polar", "start.json"), "utf8"),
);
const given = process.argv.slice(2);
const dashes = given.indexOf("--");
const extra = dashes < 0 ? given : given.slice(dashes + 1);
let scratch;
let args;

if (config.manifest) {
  const { manifest } = config;
  const hosts = Object.fromEntries(
    Object.entries(manifest.hosts).map(([host, dir]) => [host, resolve(here, dir)]),
  );
  const path = join((scratch = mkdtempSync(join(tmpdir(), "polar-start-"))), "manifest.json");

  writeFileSync(
    path,
    JSON.stringify({ ...manifest, root: resolve(here, manifest.root), out: here, hosts }),
  );
  args = [path];
} else {
  args = [join(here, config.main), config.source, join(here, "_polar", "runtime.js")];
}

const child = spawn(
  process.execPath,
  ["--enable-source-maps", join(here, config.launcher), ...args, "--", ...extra],
  { stdio: "inherit" },
);

process.on("SIGINT", () => {});
process.on("SIGTERM", () => child.kill("SIGTERM"));

child.on("exit", (code, signal) => {
  if (scratch) {
    rmSync(scratch, { recursive: true, force: true });
  }

  process.exitCode = code ?? (signal ? 1 : 0);
});
