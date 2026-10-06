export function now() {
  return { $: "Time", _0: Date.now() };
}

const pad = (n, width) => String(n).padStart(width, "0");

export function format_iso(ms) {
  const d = new Date(ms);

  return (
    `${pad(d.getUTCFullYear(), 4)}-${pad(d.getUTCMonth() + 1, 2)}-` +
    `${pad(d.getUTCDate(), 2)}T${pad(d.getUTCHours(), 2)}:` +
    `${pad(d.getUTCMinutes(), 2)}:${pad(d.getUTCSeconds(), 2)}.` +
    `${pad(d.getUTCMilliseconds(), 3)}Z`
  );
}

export function format_compact(ms) {
  const d = new Date(ms);

  return (
    `${pad(d.getUTCFullYear(), 4)}${pad(d.getUTCMonth() + 1, 2)}` +
    `${pad(d.getUTCDate(), 2)}_${pad(d.getUTCHours(), 2)}` +
    `${pad(d.getUTCMinutes(), 2)}${pad(d.getUTCSeconds(), 2)}`
  );
}

const SHAPE =
  /^(\d{4})-(\d{2})-(\d{2})(?:T|\s)(\d{2}):(\d{2}):(\d{2})(?:\.(\d{1,9}))?(Z|[+-]\d{2}:\d{2})?$/;

export function parse_iso(text) {
  const m = SHAPE.exec(text);

  if (m === null) {
    return null;
  }

  const zone = m[8];
  const spaced = text[10] === " ";

  if (zone === undefined && !spaced) {
    return null;
  }

  if (zone !== undefined && spaced) {
    return null;
  }

  const [year, month, day, hour, minute, second] = m
    .slice(1, 7)
    .map(Number);
  const millis = m[7] === undefined ? 0 : Number(m[7].padEnd(3, "0").slice(0, 3));

  let offset = 0;

  if (zone !== undefined && zone !== "Z") {
    const oh = Number(zone.slice(1, 3));
    const om = Number(zone.slice(4, 6));

    if (oh > 23 || om > 59) {
      return null;
    }

    offset = (oh * 60 + om) * (zone[0] === "-" ? -1 : 1);
  }

  if (hour > 23 || minute > 59 || second > 59) {
    return null;
  }

  const date = new Date(0);

  date.setUTCFullYear(year, month - 1, day);
  date.setUTCHours(hour, minute, second, millis);

  if (
    date.getUTCFullYear() !== year ||
    date.getUTCMonth() !== month - 1 ||
    date.getUTCDate() !== day
  ) {
    return null;
  }

  return date.getTime() - offset * 60000;
}
