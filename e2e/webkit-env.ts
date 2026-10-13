// The test browser's WebKit runs with its BBQ wasm compiler off (#43).
//
// Playwright's Linux WebKit build (26.0, Playwright 1.58.2) now and then
// traps ("Unreachable code…", "access to a null reference", "Out of
// bounds memory access") or crashes its page process while the surface's
// wasm loads. The on-demand stress run (`wasm-stress.yml`) pinned it on the
// BBQ tier: under its early tier-up amplifier, 7 traps and 4 crashes in
// 17 500 loads with BBQ on, 4 traps with OMG off, and 0 traps and 0 crashes
// in 16 000 loads with BBQ off (#43 comment 6136758501). The real iPad's
// Safari has shown neither. BBQ off changes how the test browser compiles
// wasm (a load is ~15-25 % slower), not what the tests check. Revisit at
// each Playwright update: run the stress baseline with BBQ on again and
// drop this when it is clean.
//
// Playwright hands `env` to the browser it launches and its page processes
// inherit it; JavaScriptCore reads `JSC_*` at start on Linux (the stress
// run's page-process check confirmed it).
export const WEBKIT_ENV: Record<string, string> = {
  ...Object.fromEntries(
    Object.entries(process.env).filter((entry): entry is [string, string] => entry[1] !== undefined),
  ),
  JSC_useBBQJIT: "0",
};
