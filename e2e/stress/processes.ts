import fs from "node:fs";

// The WebKit page processes on this machine and the JavaScriptCore options
// in their environment (#43): JavaScriptCore reads `JSC_<option>=<value>`
// from its environment at start (Linux), so what a page process holds is
// what its engine runs with. Read from /proc, as crash.spec.ts finds the
// processes it signals.

/** The processes under `root` (itself excluded), read from /proc. */
export function descendants(root: number): number[] {
  const children = new Map<number, number[]>();
  for (const name of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(name)) continue;
    let stat: string;
    try {
      stat = fs.readFileSync(`/proc/${name}/stat`, "utf8");
    } catch {
      continue; // the process ended while the list was read
    }
    const parent = Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]);
    const list = children.get(parent) ?? [];
    list.push(Number(name));
    children.set(parent, list);
  }
  const found: number[] = [];
  const queue = [...(children.get(root) ?? [])];
  while (queue.length > 0) {
    const pid = queue.shift()!;
    found.push(pid);
    queue.push(...(children.get(pid) ?? []));
  }
  return found;
}

/** WebKit's web (page content) processes under `root`: WPE's, or WebKitGTK's. */
export function webProcesses(root: number): number[] {
  return descendants(root).filter((pid) => {
    try {
      return /\/(WPEWebProcess|WebKitWebProcess)$/.test(fs.readFileSync(`/proc/${pid}/cmdline`, "utf8").split("\0")[0]);
    } catch {
      return false; // ended meanwhile
    }
  });
}

/** The `JSC_*` variables of an environment, sorted by name. */
export function jscOf(env: Record<string, string | undefined>): Record<string, string> {
  return Object.fromEntries(
    Object.entries(env)
      .filter((entry): entry is [string, string] => entry[0].startsWith("JSC_") && entry[1] !== undefined)
      .sort(([a], [b]) => a.localeCompare(b)),
  );
}

/** The `JSC_*` variables process `pid` started with (its /proc environ). */
export function jscEnv(pid: number): Record<string, string> {
  const env: Record<string, string> = {};
  for (const entry of fs.readFileSync(`/proc/${pid}/environ`, "utf8").split("\0")) {
    const at = entry.indexOf("=");
    if (at > 0) env[entry.slice(0, at)] = entry.slice(at + 1);
  }
  return jscOf(env);
}

/** This process's environment without its unset entries (a browser launch's `env`). */
export function ownEnv(): Record<string, string> {
  return Object.fromEntries(
    Object.entries(process.env).filter((entry): entry is [string, string] => entry[1] !== undefined),
  );
}
