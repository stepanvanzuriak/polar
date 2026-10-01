import { readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";

const file = () => process.env.POLAR_STORAGE_FILE ?? ".polar-storage.json";

const load = () => {
  try {
    return JSON.parse(readFileSync(file(), "utf8"));
  } catch (error) {
    if (error.code === "ENOENT") {
      return {};
    }

    throw error;
  }
};

const save = (data) => {
  const temp = `${file()}.${process.pid}.tmp`;

  writeFileSync(temp, `${JSON.stringify(data, null, 2)}\n`);
  renameSync(temp, file());
};

export function get(key) {
  const data = load();

  return Object.hasOwn(data, key) ? String(data[key]) : null;
}

export function set(key, value) {
  save({ ...load(), [key]: value });
}

export function remove(key) {
  const data = load();

  if (Object.hasOwn(data, key)) {
    delete data[key];
    save(data);
  }
}

export function keys() {
  return Object.keys(load());
}

export function clear() {
  rmSync(file(), { force: true });
}
