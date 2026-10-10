import { ASYNC } from "../../runtime.js";
import { spawn } from "node:child_process";
import { constants } from "node:os";

export function args() {
  const at = process.argv.indexOf("--");

  return at < 0 ? [] : process.argv.slice(at + 1);
}

export function env(name) {
  return Object.hasOwn(process.env, name) ? process.env[name] : null;
}

export function cwd() {
  return process.cwd();
}

export function set_exit_code(code) {
  process.exitCode = code;
}

export async function run(command, args, options) {
  const { code } = await start(command, args, options, "inherit");
  return code;
}

export function output(command, args, options) {
  return start(command, args, options, ["inherit", "pipe", "pipe"]);
}

function start(command, args, options, stdio) {
  return new Promise((resolve) => {
    const ignore = () => {};
    const stdout = [];
    const stderr = [];
    let done = false;
    const finish = (code) => {
      if (done) {
        return;
      }

      done = true;
      process.off("SIGINT", ignore);
      resolve({
        code,
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8"),
      });
    };

    process.on("SIGINT", ignore);

    const child = spawn(command, args, {
      cwd: options.cwd ?? undefined,
      env: {
        ...process.env,
        ...Object.fromEntries(
          options.env.map(({ name, value }) => [name, value]),
        ),
      },
      stdio,
    });

    child.stdout?.on("data", (chunk) => stdout.push(chunk));
    child.stderr?.on("data", (chunk) => stderr.push(chunk));
    child.on("error", (error) => {
      process.stderr.write(
        `error: cannot run \`${command}\`: ${reason(error)}\n`,
      );
      finish(127);
    });
    child.on("close", (code, signal) => {
      finish(code ?? 128 + (constants.signals[signal] ?? 0));
    });
  });
}

function reason(error) {
  switch (error.code) {
    case "ENOENT":
      return "command not found";
    case "EACCES":
      return "permission denied";
    default:
      return error.message;
  }
}

export async function scoped_env(vars, body) {
  const saved = vars.map(({ name }) => [
    name,
    Object.hasOwn(process.env, name) ? process.env[name] : undefined,
  ]);

  for (const { name, value } of vars) {
    process.env[name] = value;
  }

  try {
    return await body(ASYNC);
  } finally {
    for (const [name, value] of saved) {
      if (value === undefined) {
        delete process.env[name];
      } else {
        process.env[name] = value;
      }
    }
  }
}

const children = new Map();
let nextChild = 1;

export function spawn_child(command, args, options) {
  const lines = [];
  const waiting = [];
  const child = spawn(command, args, {
    cwd: options.cwd ?? undefined,
    env: {
      ...process.env,
      ...Object.fromEntries(options.env.map(({ name, value }) => [name, value])),
    },
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
  });
  const entry = { child, lines, waiting, closed: false, code: null };
  const settle = () => {
    for (const waiter of [...waiting]) {
      waiter();
    }
  };

  for (const stream of [child.stdout, child.stderr]) {
    let rest = "";

    stream.setEncoding("utf8");
    stream.on("data", (chunk) => {
      const parts = (rest + chunk).split("\n");

      rest = parts.pop();
      lines.push(...parts);
      settle();
    });
    stream.on("end", () => {
      if (rest !== "") {
        lines.push(rest);
        rest = "";
      }

      settle();
    });
  }

  child.on("error", () => {
    entry.closed = true;
    entry.code = 127;
    settle();
  });
  child.on("close", (code, signal) => {
    entry.closed = true;
    entry.code = code ?? 128 + (constants.signals[signal] ?? 0);
    settle();
  });

  const id = nextChild++;

  children.set(id, entry);

  return { pid: child.pid ?? 0, id };
}

export async function wait_for_line(handle, pattern, timeoutMs) {
  const entry = children.get(handle.id);

  if (entry === undefined) {
    return null;
  }

  const re = new RegExp(pattern, "u");
  let seen = 0;

  return new Promise((resolve) => {
    let timer;
    const check = () => {
      for (; seen < entry.lines.length; seen++) {
        if (re.test(entry.lines[seen])) {
          return done(entry.lines[seen++]);
        }
      }

      if (entry.closed) {
        done(null);
      }
    };
    const done = (line) => {
      clearTimeout(timer);
      entry.waiting.splice(entry.waiting.indexOf(check), 1);
      resolve(line);
    };

    timer = setTimeout(() => done(null), timeoutMs);
    entry.waiting.push(check);
    check();
  });
}

export async function stop(handle) {
  const entry = children.get(handle.id);

  if (entry === undefined) {
    return 0;
  }

  if (!entry.closed) {
    const exited = new Promise((resolve) => {
      const check = () => {
        if (entry.closed) {
          entry.waiting.splice(entry.waiting.indexOf(check), 1);
          resolve();
        }
      };

      entry.waiting.push(check);
    });

    try {
      process.kill(-entry.child.pid, "SIGINT");
    } catch {
      entry.child.kill("SIGINT");
    }

    await exited;
  }

  children.delete(handle.id);

  return entry.code;
}
