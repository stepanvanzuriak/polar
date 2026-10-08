import {
  createHash,
  createHmac,
  randomBytes,
  timingSafeEqual,
  scrypt,
} from "node:crypto";
import { promisify } from "node:util";

const scryptAsync = promisify(scrypt);

const DEFAULTS = { ln: 15, r: 8, p: 1 };
const MAX_LN = 16;
const MAX_R = 16;
const MAX_P = 16;
const HEX = /^(?:[0-9a-f]{2})+$/;
const KEYLEN = 32;

export function bytes(count) {
  return randomBytes(count).toString("hex");
}

export function digest_sha256(text) {
  return createHash("sha256").update(text, "utf8").digest("hex");
}

export function digest_hmac_sha256(key, text) {
  return createHmac("sha256", Buffer.from(key, "utf8"))
    .update(text, "utf8")
    .digest("hex");
}

export function timing_safe_equal(a, b) {
  const left = Buffer.from(a, "utf8");
  const right = Buffer.from(b, "utf8");

  return left.length === right.length && timingSafeEqual(left, right);
}

export function base64url_encode(text) {
  return Buffer.from(text, "utf8").toString("base64url");
}

const decoder = new TextDecoder("utf-8", { fatal: true });

export function base64url_decode(text) {
  if (!/^[A-Za-z0-9_-]*$/.test(text) || text.length % 4 === 1) {
    return null;
  }

  const raw = Buffer.from(text, "base64url");

  if (raw.toString("base64url") !== text) {
    return null;
  }

  try {
    return decoder.decode(raw);
  } catch {
    return null;
  }
}

async function derive(plain, saltHex, { ln, r, p }) {
  const N = 2 ** ln;
  return scryptAsync(plain, Buffer.from(saltHex, "hex"), KEYLEN, {
    N,
    r,
    p,
    maxmem: 256 * N * r,
  });
}

export async function scrypt_hash(plain, saltHex) {
  const { ln, r, p } = DEFAULTS;
  const hash = (await derive(plain, saltHex, DEFAULTS)).toString("hex");
  return `$scrypt$ln=${ln},r=${r},p=${p}$${saltHex}$${hash}`;
}

function parse(stored) {
  const parts = stored.split("$");

  if (parts.length !== 5 || parts[0] !== "" || parts[1] !== "scrypt") {
    return null;
  }

  const params = {};

  for (const pair of parts[2].split(",")) {
    const [key, value, ...rest] = pair.split("=");

    if (rest.length > 0 || !/^\d+$/.test(value ?? "")) {
      return null;
    }

    params[key] = Number(value);
  }

  const { ln, r, p } = params;
  const salt = parts[3];
  const hash = parts[4];

  if (![ln, r, p].every(Number.isSafeInteger)) {
    return null;
  }

  if (ln < 1 || r < 1 || p < 1 || ln > MAX_LN || r > MAX_R || p > MAX_P) {
    return null;
  }

  if (!HEX.test(salt) || !HEX.test(hash)) {
    return null;
  }

  return { ln, r, p, salt, hash };
}

export async function scrypt_verify(plain, stored) {
  try {
    const d = parse(stored);
    if (!d) {
      return false;
    }
    const actual = await derive(plain, d.salt, d);
    const expected = Buffer.from(d.hash, "hex");
    return (
      actual.length === expected.length && timingSafeEqual(actual, expected)
    );
  } catch {
    return false;
  }
}

export function scrypt_needs_rehash(stored) {
  const d = parse(stored);
  return !d ? true : d.ln < DEFAULTS.ln || d.r < DEFAULTS.r || d.p < DEFAULTS.p;
}
