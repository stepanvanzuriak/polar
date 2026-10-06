const cache = new Map();

function compiled(pattern) {
  let re = cache.get(pattern);

  if (re === undefined) {
    try {
      re = new RegExp(pattern, "u");
    } catch {
      re = null;
    }

    cache.set(pattern, re);
  }

  return re;
}

export function syntax_error(pattern) {
  try {
    new RegExp(pattern, "u");
    return null;
  } catch (error) {
    return error.message;
  }
}

export function test_pattern(pattern, text) {
  const re = compiled(pattern);

  return re !== null && re.test(text);
}

export function first_match(pattern, text) {
  const re = compiled(pattern);

  if (re === null) {
    return null;
  }

  const m = re.exec(text);

  return m === null ? null : m[0];
}

export function all_matches(pattern, text) {
  const re = compiled(pattern);

  if (re === null) {
    return [];
  }

  const global = new RegExp(re.source, "gu");
  const out = [];

  for (const m of text.matchAll(global)) {
    out.push(m[0]);
  }

  return out;
}

export function replace_all(pattern, text, replacement) {
  const re = compiled(pattern);

  if (re === null) {
    return text;
  }

  return text.replace(new RegExp(re.source, "gu"), () => replacement);
}
