const decoder = new TextDecoder("utf-8", { fatal: true });
const encoder = new TextEncoder();

function clamp(position, size) {
  return Math.max(0, Math.min(position, size));
}

export function length(bytes) {
  return bytes.length;
}

export function slice(bytes, from, to) {
  const start = clamp(from, bytes.length);
  const end = clamp(to, bytes.length);

  return start < end ? bytes.slice(start, end) : new Uint8Array(0);
}

export function index_of(bytes, needle, from) {
  const found = Buffer.from(bytes.buffer, bytes.byteOffset, bytes.length).indexOf(
    needle,
    clamp(from, bytes.length),
  );

  return found < 0 ? null : found;
}

export function to_string(bytes) {
  try {
    return { ok: decoder.decode(bytes) };
  } catch {
    return { err: "bytes are not valid UTF-8" };
  }
}

export function of_string(text) {
  return encoder.encode(text);
}

export function concat(a, b) {
  const out = new Uint8Array(a.length + b.length);

  out.set(a, 0);
  out.set(b, a.length);

  return out;
}

export function from_list(values) {
  return Uint8Array.from(values, (value) => value & 0xff);
}

export function to_list(bytes) {
  return Array.from(bytes);
}

export function equal(a, b) {
  return a.length === b.length && a.every((byte, i) => byte === b[i]);
}
