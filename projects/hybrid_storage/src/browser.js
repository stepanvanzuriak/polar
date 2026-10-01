const memory = new Map();

const fallback = {
  getItem: (key) => (memory.has(key) ? memory.get(key) : null),
  setItem: (key, value) => memory.set(key, value),
  removeItem: (key) => memory.delete(key),
  key: (index) => [...memory.keys()][index] ?? null,
  clear: () => memory.clear(),
  get length() {
    return memory.size;
  },
};

const store = () => globalThis.window?.localStorage ?? fallback;

export function get(key) {
  return store().getItem(key);
}

export function set(key, value) {
  store().setItem(key, value);
}

export function remove(key) {
  store().removeItem(key);
}

export function keys() {
  const local = store();

  return Array.from({ length: local.length }, (_, i) => local.key(i));
}

export function clear() {
  store().clear();
}
