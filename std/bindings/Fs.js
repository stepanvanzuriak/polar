import * as fs from "node:fs/promises";

export function read(path) {
  return attempt(path, () => fs.readFile(path, "utf8"));
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
