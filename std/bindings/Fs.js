import { ASYNC } from "../../runtime.js";
import * as fs from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

export function read(path) {
  return attempt(path, () => fs.readFile(path, "utf8"));
}

export function read_bytes(path) {
  return attempt(path, async () => new Uint8Array(await fs.readFile(path)));
}

export function write_bytes(path, bytes) {
  return attempt(path, () => fs.writeFile(path, bytes));
}

export function write(path, text) {
  return attempt(path, () => fs.writeFile(path, text, "utf8"));
}

export function append(path, text) {
  return attempt(path, () => fs.appendFile(path, text, "utf8"));
}

export async function exists(path) {
  try {
    await fs.stat(path);
    return true;
  } catch {
    return false;
  }
}

export async function is_dir(path) {
  try {
    return (await fs.stat(path)).isDirectory();
  } catch {
    return false;
  }
}

export function mkdir_all(path) {
  return attempt(path, () => fs.mkdir(path, { recursive: true }));
}

export function list(path) {
  return attempt(path, async () => sorted(await fs.readdir(path)));
}

export function walk(path) {
  return attempt(path, async () => sorted(await files(path, "")));
}

export function copy(from, to) {
  return attempt(from, () =>
    fs.cp(from, to, {
      recursive: true,
      force: true,
      verbatimSymlinks: true,
      preserveTimestamps: true,
    }),
  );
}

export function symlink(target, path) {
  return attempt(path, () => fs.symlink(target, path));
}

export function set_mtime(path, millis) {
  return attempt(path, () => fs.utimes(path, millis / 1000, millis / 1000));
}

export function chmod(path, mode) {
  return attempt(path, () => fs.chmod(path, mode));
}

export function remove(path) {
  return attempt(path, async () => {
    const stat = await fs.lstat(path);

    if (stat.isDirectory()) {
      await fs.rmdir(path);
    } else {
      await fs.unlink(path);
    }
  });
}

export function remove_all(path) {
  return attempt(path, () => fs.rm(path, { recursive: true, force: true }));
}

export function mkdtemp(prefix) {
  return attempt(prefix, () => fs.mkdtemp(join(tmpdir(), prefix)));
}

export function temp_dir(prefix) {
  return mkdtemp(prefix);
}

export async function with_temp_dir(prefix, body) {
  const dir = await fs.mkdtemp(join(tmpdir(), prefix));

  try {
    return await body(dir, ASYNC);
  } finally {
    await fs.rm(dir, { recursive: true, force: true });
  }
}

async function files(root, prefix) {
  const out = [];
  const entries = await fs.readdir(prefix ? `${root}/${prefix}` : root, {
    withFileTypes: true,
  });

  for (const entry of entries) {
    const name = prefix ? `${prefix}/${entry.name}` : entry.name;

    if (entry.isDirectory()) {
      out.push(...(await files(root, name)));
    } else {
      out.push(name);
    }
  }

  return out;
}

function sorted(names) {
  return names.sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
}

async function attempt(path, action) {
  try {
    const value = await action();
    return { ok: value ?? {} };
  } catch (error) {
    return {
      err: {
        code: error?.code ?? "EUNKNOWN",
        path: error?.path ?? path,
        message: error?.message ?? String(error),
      },
    };
  }
}
