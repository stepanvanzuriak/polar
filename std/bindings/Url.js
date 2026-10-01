export function decode(text) {
  try {
    return decodeURIComponent(text);
  } catch {
    return text;
  }
}

export function encode(text) {
  return encodeURIComponent(text);
}
