import { test, expect } from "./support/fixtures";
import { openSurface } from "./support/live";

// The name button's corner (#127 review I1, I2; the approved mockup
// docs/mockups/paired-mute-v3.html, variant A): a problem's `!`, the linked
// mute's chain and a return's RET in the top-left corner, the MUTE mark at
// the top right, the name under them. When they do not all fit the strip's
// width, the MUTE mark yields first (the dark strip, and on a linked strip
// the red chain, say "muted"), then RET; `!` and the chain never yield,
// whatever their count: a count of four characters or more (a mixed count of
// a two-digit total, "1/10" to "12/12") is drawn compact (#127 re-review N1).
// The fader link's mark (#130, D18; mockup reverb-link-v2: a leader's
// filled, a partner's outlined) comes right after the chain and before
// RET: the chain alone beside a crowded mute chain (with `!`, or a count of
// two characters or more), so it shows at every width but beside both `!`
// and a mute chain, where it shows exactly when it fits (64 px never), and
// RET and the MUTE mark yield before it.
//
// A static render of the strip head as StripView and MuteView draw it, in
// the surface's own stylesheets and bundled fonts, swept over the strip
// widths: every link state (all playing, mixed 1/3 and 2/4, all muted; and
// with a two-digit total: all playing and all muted 10 and 12, mixed 1/10,
// 9/12, 10/12 and 12/12) with and without `!` and RET, and unlinked strips
// (playing and muted, with and without `!` and RET), each with a short and a
// long name, each with no fader mark, a leader's and a partner's.

const WIDTHS = [64, 72, 76, 88, 120];
const LABELS = ["Vox 1", "Klavir 2"];
/** A link's state (the chain's `data-state` and text), or none: an unlinked strip. */
const LINKS: ({ state: "all" | "mixed" | "muted"; text: string } | null)[] = [
  null,
  { state: "all", text: "3" },
  { state: "mixed", text: "1/3" },
  { state: "mixed", text: "2/4" },
  { state: "muted", text: "3" },
  // Two-digit totals (#127 re-review N1): `!` and the chain still fit.
  { state: "all", text: "10" },
  { state: "all", text: "12" },
  { state: "muted", text: "10" },
  { state: "muted", text: "12" },
  { state: "mixed", text: "1/10" },
  { state: "mixed", text: "9/12" },
  { state: "mixed", text: "10/12" },
  { state: "mixed", text: "12/12" },
];

/** The fader link's mark (#130): none, a leader's or a partner's. */
const FADERS: ("" | "lead" | "follow")[] = ["", "lead", "follow"];

type Case = {
  w: number;
  label: string;
  link: { state: "all" | "mixed" | "muted"; text: string } | null;
  flag: boolean;
  ret: boolean;
  /** An unlinked strip's mute (a linked strip's follows its link). */
  muted: boolean;
  fader: "" | "lead" | "follow";
};

function cases(): Case[] {
  const out: Case[] = [];
  for (const w of WIDTHS)
    for (const label of LABELS)
      for (const link of LINKS)
        for (const flag of [false, true])
          for (const ret of [false, true])
            for (const fader of FADERS)
              for (const muted of link ? [false] : [false, true]) out.push({ w, label, link, flag, ret, muted, fader });
  return out;
}

/**
 * Draws each case's strip head in the page, over the surface, as StripView
 * and MuteView draw it, and measures it: the faces not loaded, every
 * problem, and how many MUTE marks and RETs yielded at each width.
 */
async function sweep(
  list: Case[],
): Promise<{ faces: string[]; problems: string[]; yielded: Record<number, { mute: number; ret: number; fader: number }> }> {
  const host = document.createElement("div");
  host.id = "corner-sweep";
  host.style.cssText =
    "position:fixed;inset:0;z-index:2147483647;overflow:auto;display:flex;flex-wrap:wrap;align-items:flex-start;gap:6px;padding:6px;background:#0b0b12";
  document.body.appendChild(host);
  /** LinkMark's `compact` (behave::mute::chain_compact): a count of four characters or more. */
  const compact = (text: string) => [...text].length >= 4;
  /** The fader mark's `compact` (behave::fader_link::flink_compact): beside `!` and a mute chain, or a count of two characters or more. */
  const crowded = (c: Case) => !!c.link && (c.flag || [...c.link.text].length >= 2);
  const strips = list.map((c) => {
    const muted = c.link ? c.link.state === "muted" : c.muted;
    const attr = c.link ? { all: "false", mixed: "mixed", muted: "true" }[c.link.state] : String(c.muted);
    const flag = c.flag ? '<span class="strip-flag" aria-hidden="true">!</span>' : "";
    const chain = c.link
      ? `<span class="link-mark${compact(c.link.text) ? " compact" : ""}" data-testid="mute-link" data-state="${c.link.state}" aria-hidden="true"><span class="link-n" data-testid="mute-link-n">${c.link.text}</span></span>`
      : "";
    const fader = c.fader
      ? `<span class="link-mark flink${c.fader === "follow" ? " follow" : ""}${crowded(c) ? " compact" : ""}" data-testid="fader-link-mark" data-role="${c.fader}" aria-hidden="true"><span class="link-n">dB</span></span>`
      : "";
    const ret = c.ret ? '<span class="strip-ret" data-testid="strip-ret" aria-hidden="true">RET</span>' : "";
    const s = document.createElement("div");
    s.className = "strip";
    s.setAttribute("data-testid", "strip");
    if (c.flag) s.setAttribute("data-mark", "problem");
    s.setAttribute("data-flink", c.fader);
    s.style.setProperty("--strip-w", `${c.w}px`);
    s.innerHTML =
      `<div class="strip-head">` +
      `<div class="mute${muted ? "" : " lit"}" data-testid="mute" data-muted="${attr}" data-binding="ready" aria-disabled="false">` +
      `<span class="strip-label" data-testid="strip-label" style="--n:${[...c.label].length};">${c.label}</span>` +
      `<span class="strip-corner">${flag}${chain}${fader}${ret}<span class="mute-mark" aria-hidden="true">MUTE</span></span>` +
      `</div>` +
      `<div class="strip-info"><div class="strip-readout" data-testid="strip-readout"><span class="status"></span>` +
      `<span class="db" data-testid="db">−12.0</span>${c.flag ? '<span class="strip-mark" data-testid="strip-mark">ZNAČKA?</span>' : ""}</div>` +
      `<button type="button" class="strip-menu" data-testid="strip-menu"><span class="strip-menu-icon" aria-hidden="true">☰</span></button></div>` +
      `</div>`;
    host.appendChild(s);
    return s;
  });
  // The bundled faces, loaded after a forced layout (`fonts.ready` read
  // right after the markup can resolve before a load starts).
  void host.offsetHeight;
  await Promise.all([...document.fonts].map((f) => f.load().catch(() => f)));
  const faces = [...document.fonts].filter((f) => f.status !== "loaded").map((f) => `${f.family} ${f.weight} ${f.status}`);

  /** An element's box and whether it can be seen: drawn, and inside every box that clips it up to the strip. */
  const look = (el: Element | null, strip: Element) => {
    if (!el) return null;
    const b = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    let seen = cs.display !== "none" && cs.visibility === "visible" && b.width > 0 && b.height > 0;
    for (let a = el.parentElement; a && a !== strip.parentElement; a = a.parentElement) {
      if (getComputedStyle(a).overflow === "visible") continue;
      const c = a.getBoundingClientRect();
      if (b.left < c.left - 0.5 || b.right > c.right + 0.5 || b.top < c.top - 0.5 || b.bottom > c.bottom + 0.5) seen = false;
    }
    return { left: b.left, right: b.right, top: b.top, width: b.width, seen };
  };
  const problems: string[] = [];
  const yielded: Record<number, { mute: number; ret: number; fader: number }> = {};
  for (const c of list) yielded[c.w] = { mute: 0, ret: 0, fader: 0 };
  list.forEach((c, i) => {
    const s = strips[i];
    const name = `${c.w} px "${c.label}" ${c.link ? `${c.link.state} ${c.link.text}` : c.muted ? "unlinked muted" : "unlinked"}${c.flag ? " !" : ""}${c.fader ? ` ${c.fader}` : ""}${c.ret ? " RET" : ""}`;
    const fail = (what: string) => problems.push(`${name}: ${what}`);
    const at = (n: number) => n.toFixed(2);
    const button = s.querySelector(".mute")!.getBoundingClientRect();
    const strip = s.getBoundingClientRect();
    // The corner's line: the button less 3 px each side.
    const left = button.left + 3;
    const right = button.right - 3;
    const label = s.querySelector(".strip-label") as HTMLElement;
    const range = document.createRange();
    range.selectNodeContents(label);
    const text = range.getBoundingClientRect();
    const lb = label.getBoundingClientRect();
    if (label.scrollWidth > label.clientWidth || text.left < lb.left - 0.5 || text.right > lb.right + 0.5) {
      fail(`the label is clipped (${label.scrollWidth} > ${label.clientWidth}, text ${at(text.left)}..${at(text.right)} in ${at(lb.left)}..${at(lb.right)})`);
    }
    const flag = look(s.querySelector(".strip-flag"), s);
    const chain = look(s.querySelector(".link-mark:not(.flink)"), s);
    const fmark = look(s.querySelector(".link-mark.flink"), s);
    const ret = look(s.querySelector(".strip-ret"), s);
    const mute = look(s.querySelector(".mute-mark"), s)!;
    // `!` first, then the chain: never hidden, where they were.
    if (flag) {
      if (!flag.seen) fail("`!` is hidden");
      if (Math.abs(flag.left - button.left - 3) > 0.5 || Math.abs(flag.top - button.top - 3) > 0.5) {
        fail(`\`!\` at (${at(flag.left - button.left)}, ${at(flag.top - button.top)}), not (3, 3)`);
      }
    }
    // The corner's right end as the marks before it leave it.
    let corner = left;
    if (flag) corner = left + flag.width;
    if (chain) {
      if (!chain.seen) fail("the chain is hidden");
      const want = flag ? corner + 3 : left;
      if (Math.abs(chain.left - want) > 0.5 || chain.top - button.top > 3) {
        fail(`the chain at (${at(chain.left - button.left)}, ${at(chain.top - button.top)}), not the top-left corner`);
      }
      corner = want + chain.width;
    }
    // One flex line: once a mark wraps out, every mark after it does.
    let wrapped = false;
    // The fader mark follows them: always where it fits, and it fits but
    // beside both `!` and a mute chain.
    if (fmark) {
      const start = flag || chain ? corner + 3 : left;
      const fits = start + fmark.width <= right + 0.01;
      if (fmark.seen !== fits) fail(`the fader mark ${fmark.seen ? "shows" : "is hidden"} with the corner to ${at(start + fmark.width - button.left)} in ${at(right - button.left)}`);
      if (!fits && !(c.flag && c.link)) fail(`the fader mark is hidden (${at(fmark.width)} px from ${at(start - button.left)} in ${at(right - button.left)})`);
      if (fmark.seen) {
        if (Math.abs(fmark.left - start) > 0.5 || fmark.top - button.top > 3) {
          fail(`the fader mark at (${at(fmark.left - button.left)}, ${at(fmark.top - button.top)}), not after the marks before it`);
        }
        corner = start + fmark.width;
      } else {
        wrapped = true;
        yielded[c.w].fader += 1;
      }
    }
    // RET follows them while the corner fits the button, else it yields.
    if (ret) {
      const start = flag || chain || fmark?.seen ? corner + 3 : left;
      const fits = !wrapped && start + ret.width <= right + 0.01;
      if (ret.seen !== fits) fail(`RET ${ret.seen ? "shows" : "is hidden"} with the corner to ${at(start + ret.width - button.left)} in ${at(right - button.left)}`);
      if (!c.link && !c.fader && !fits) fail("an unlinked strip's RET is hidden");
      if (ret.seen) {
        if (Math.abs(ret.left - start) > 0.5 || Math.abs(ret.top - button.top - 3) > 0.5) {
          fail(`RET at (${at(ret.left - button.left)}, ${at(ret.top - button.top)}), not (${at(start - button.left)}, 3)`);
        }
        corner = start + ret.width;
      } else {
        wrapped = true;
        yielded[c.w].ret += 1;
      }
    }
    const hasCorner = !!flag || !!chain || !!fmark?.seen || !!ret?.seen;
    if (hasCorner && corner > right + 0.5) fail(`the corner ends at ${at(corner - button.left)}, past the button's ${at(right - button.left)}`);
    // The MUTE mark of an all-muted strip: at the top right as before
    // #127 while it fits beside the corner, else it yields.
    const muted = c.link ? c.link.state === "muted" : c.muted;
    if (!muted && mute.seen) fail("the MUTE mark shows on a strip that plays");
    if (muted) {
      const fits = !wrapped && (hasCorner ? corner + 3 : left) + mute.width <= right + 0.01;
      if (mute.seen !== fits) fail(`the MUTE mark ${mute.seen ? "shows" : "is hidden"} beside a corner to ${at(corner - button.left)} (MUTE ${at(mute.width)} wide, the line to ${at(right - button.left)})`);
      if (mute.seen) {
        if (Math.abs(mute.right - right) > 0.5 || Math.abs(mute.top - button.top - 2) > 0.5) {
          fail(`the MUTE mark at (${at(mute.left - button.left)}, ${at(mute.top - button.top)}), not at the top right`);
        }
        if (hasCorner && corner + 3 > mute.left + 0.5) fail(`the corner ends at ${at(corner - button.left)}, on the MUTE mark at ${at(mute.left - button.left)}`);
      } else {
        yielded[c.w].mute += 1;
      }
    }
    // Nothing passes the strip's right edge.
    for (const el of s.querySelectorAll("*")) {
      const r = el.getBoundingClientRect();
      if ((r.width > 0 || r.height > 0) && r.right > strip.right + 0.5) fail(`.${el.className} ends at ${at(r.right - strip.left)}, past the strip's ${at(strip.width)}`);
    }
  });
  host.remove();
  return { faces, problems, yielded };
}

test.describe("The name button's corner", () => {
  test("every mark fits the strip from 64 to 120 px: the MUTE mark yields first, then RET, then the fader mark beside `!` and a chain; never `!` or the chain", async ({ page }) => {
    await openSurface(page);
    const result = await page.evaluate(sweep, cases());
    expect(result.faces, "the bundled faces loaded").toEqual([]);
    test.info().annotations.push({ type: "yielded", description: JSON.stringify(result.yielded) });
    console.log(`corner sweep (${test.info().project.name}): yielded ${JSON.stringify(result.yielded)}`);
    expect(result.problems).toEqual([]);
    // The sweep reaches every yield at 64 px (the fader mark only beside
    // `!` and a chain), none of the fader mark from 88 px, and at 120 px
    // every mark shows.
    expect(result.yielded[64].mute).toBeGreaterThan(0);
    expect(result.yielded[64].ret).toBeGreaterThan(0);
    expect(result.yielded[64].fader).toBeGreaterThan(0);
    expect(result.yielded[88].fader).toBe(0);
    expect(result.yielded[120]).toEqual({ mute: 0, ret: 0, fader: 0 });
  });
});
