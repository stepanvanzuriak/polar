const store = new Map();

export function get_item(key) {
  if (!store.has(key)) {
    throw {
      [Symbol.for("polar.error")]: true,
      type: "Effects.Missing",
      value: { $: "Missing", _0: key },
    };
  }

  return store.get(key);
}

export function set_item(key, value) {
  store.set(key, value);

  return {};
}
