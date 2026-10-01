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
