import { variant } from "../../runtime.js";

export function now() {
  return variant("Time", Date.now());
}

export function to_parts(ms) {
  const date = new Date(ms);

  return {
    year: date.getUTCFullYear(),
    month: date.getUTCMonth() + 1,
    day: date.getUTCDate(),
    hour: date.getUTCHours(),
    minute: date.getUTCMinutes(),
    second: date.getUTCSeconds(),
    milli: date.getUTCMilliseconds(),
  };
}

export function from_parts({ year, month, day, hour, minute, second, milli }) {
  const date = new Date(0);

  date.setUTCFullYear(year, month - 1, day);
  date.setUTCHours(hour, minute, second, milli);

  return date.getTime();
}
