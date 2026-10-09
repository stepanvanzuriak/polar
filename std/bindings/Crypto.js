import {
  createHash,
  createHmac,
  randomBytes,
  timingSafeEqual,
  scrypt as nodeScrypt,
} from "node:crypto";
import { promisify } from "node:util";

const scryptAsync = promisify(nodeScrypt);

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

export async function scrypt(plain, saltHex, ln, r, p) {
  const N = 2 ** ln;
  const hash = await scryptAsync(plain, Buffer.from(saltHex, "hex"), KEYLEN, {
    N,
    r,
    p,
    maxmem: 256 * N * r,
  });

  return hash.toString("hex");
}

export async function try_scrypt(plain, saltHex, ln, r, p) {
  try {
    return await scrypt(plain, saltHex, ln, r, p);
  } catch {
    return null;
  }
}
