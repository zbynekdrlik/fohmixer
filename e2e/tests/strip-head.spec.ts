import { test, expect } from "./support/fixtures";
import { openSurface } from "./support/live";

// The strip as the owner approved it in mockup v8 (#151,
// docs/mockups/lufs-v8.html): the name button is the strip's top edge
// (37 px, the strip's whole width), the name first and one marks line
// under it: a problem's `!`, the linked mute's chain, the fader link's mark
// and the quiet red MUTED at the right end. What gives way when the line
// fills is behave::marks::marks_look (mirrored by `look` below): a muted
// chain drops its count, with three marks the fader link drops its "dB",
// with all four the gaps narrow, and beside both `!` and a chain the fader
// link's mark yields on a narrow strip (under 78 px of its own width,
// arrange::narrow: the mockup's all four, and `!` beside a mixed two-digit
// count such as `10/12`, which leaves it no room at 64 px). `!`, the chain
// and MUTED never yield. No RET anywhere. Under the button the readout
// line (20 px: the head is 59 px, as before), then the fader zone with SEL
// at its bottom left, 3 px clear of the cap at −∞ (the main session's
// ruling on #151).
//
// A static render of the strip as StripView, MuteView, InfoColumn,
// FaderView/TrackMeter and DetailKey draw it, in the surface's own
// stylesheets and bundled fonts, swept over the strip widths: the mockup's
// combination table at 83.16 and 64 px, and every link state of #127's
// sweep (counts 3, 1/3, 2/4, 10, 12, 1/10, 9/12, 10/12, 12/12, all
// playing, mixed and muted) with and without `!` and with each fader mark,
// at every width.

const WIDTHS = [64, 72, 76, 77.99, 78, 83.16, 88, 120];
const LABELS = ["Vox 1", "Klavir 2"];
type Link = { state: "all" | "mixed" | "muted"; text: string } | null;
/** A link's state (the chain's `data-state` and text), or none: an unlinked strip. */
const LINKS: Link[] = [
  null,
  { state: "all", text: "3" },
  { state: "mixed", text: "1/3" },
  { state: "mixed", text: "2/4" },
  { state: "muted", text: "3" },
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
  link: Link;
  flag: boolean;
  /** An unlinked strip's mute (a linked strip's follows its link). */
  muted: boolean;
  fader: "" | "lead" | "follow";
  /** A conflict instead of a problem (the readout's CONFLICT). */
  conflict?: boolean;
  /** The group's name on the strip's side. */
  group?: string;
};

/** The mockup's combination table (its HEADS), at 83.16 and 64 px. */
function combinations(): Case[] {
  const base = { label: "Vox 4", flag: false, link: null as Link, muted: false, fader: "" as Case["fader"] };
  const table: Omit<Case, "w">[] = [
    { ...base },
    { ...base, link: { state: "all", text: "2" } },
    { ...base, link: { state: "muted", text: "2" } },
    { ...base, link: { state: "mixed", text: "1/2" } },
    { ...base, flag: true },
    { ...base, muted: true },
    { ...base, flag: true, link: { state: "muted", text: "12" } },
    { ...base, fader: "lead" },
    { ...base, fader: "follow", link: { state: "all", text: "2" } },
    { ...base, flag: true, link: { state: "muted", text: "12" }, fader: "lead" },
  ];
  return [83.16, 64].flatMap((w) => table.map((c) => ({ ...c, w })));
}

/** #127's sweep of every link state, `!` and fader mark, at every width. */
function sweepCases(): Case[] {
  const out: Case[] = [];
  for (const w of WIDTHS)
    for (const label of LABELS)
      for (const link of LINKS)
        for (const flag of [false, true])
          for (const fader of FADERS)
            for (const muted of link ? [false] : [false, true]) out.push({ w, label, link, flag, muted, fader });
  // A conflict's readout and a group's name at every width.
  for (const w of WIDTHS) out.push({ w, label: "Vox 1", link: null, flag: false, muted: false, fader: "", conflict: true, group: "Stage" });
  return out;
}

type Result = {
  faces: string[];
  problems: string[];
  /** Per width: the fader marks that yielded and those drawn bare. */
  yielded: Record<string, number>;
  /** The first case's measures at each width: button, head, SEL's box. */
  measures: Record<string, { button: number; head: number; top: number; width: number; strip: number; key: string }>;
};

/**
 * Draws each case's strip in the page, over the surface, as the components
 * draw it, and measures it: the faces not loaded and every problem.
 */
async function sweep(list: Case[]): Promise<Result> {
  const host = document.createElement("div");
  host.id = "strip-sweep";
  host.style.cssText =
    "position:fixed;inset:0;z-index:2147483647;overflow:auto;display:flex;flex-wrap:wrap;align-items:flex-start;gap:6px;padding:6px;background:#0b0b12";
  document.body.appendChild(host);
  /** LinkMark's `compact` (behave::mute::chain_compact): a count of four characters or more. */
  const compact = (text: string) => [...text].length >= 4;
  /** behave::marks::marks_look. */
  const look = (flag: boolean, chain: boolean, flink: boolean, muted: boolean) => {
    const n = [flag, chain, flink, muted].filter(Boolean).length;
    return { chainBare: chain && muted, flinkBare: flink && n >= 3, crowd: n >= 4, flinkYield: flink && flag && chain };
  };
  const ticks = [6, 0, -6, -12, -18, -24, -30, -40, -60];
  const strips = list.map((c) => {
    const muted = c.link ? c.link.state === "muted" : c.muted;
    const attr = c.link ? { all: "false", mixed: "mixed", muted: "true" }[c.link.state] : String(c.muted);
    const l = look(c.flag, !!c.link, !!c.fader, muted);
    const flag = c.flag ? '<span class="strip-flag" aria-hidden="true">!</span>' : "";
    const chain = c.link
      ? `<span class="link-mark${compact(c.link.text) ? " compact" : ""}${l.chainBare ? " bare" : ""}" data-testid="mute-link" data-state="${c.link.state}" aria-hidden="true"><span class="link-n" data-testid="mute-link-n">${c.link.text}</span></span>`
      : "";
    const fader = c.fader
      ? `<span class="link-mark flink${c.fader === "follow" ? " follow" : ""}${l.flinkBare ? " bare" : ""}${l.flinkYield ? " yield" : ""}" data-testid="fader-link-mark" data-role="${c.fader}" aria-hidden="true"><span class="link-n">dB</span></span>`
      : "";
    const word = c.conflict ? "CONFLICT" : c.flag ? "TAG?" : "";
    const scale = ticks
      .map((db) => `<span class="tick${db === 0 ? " unity" : ""}" style="top:${((1 - (db === 6 ? 1 : db === 0 ? 0.85 : 0.85 + db / 100)) * 100).toFixed(3)}%;">${db > 0 ? "+" + db : db < 0 ? "−" + -db : "0"}</span>`)
      .join("");
    const bar = '<div class="meter-bar" data-testid="meter-bar"><div class="meter-zones"></div><div class="meter-cover"></div><div class="meter-peak"></div></div>';
    const wrap = document.createElement("div");
    wrap.setAttribute("data-narrow", String(c.w < 78));
    wrap.setAttribute("data-narrow-wide", String(c.w * 1.1 < 78));
    wrap.style.cssText = `--strip-w:${c.w}px;display:flex;height:420px;`;
    const s = document.createElement("div");
    s.className = "strip";
    s.setAttribute("data-testid", "strip");
    if (c.conflict) s.setAttribute("data-mark", "conflict");
    else if (c.flag) s.setAttribute("data-mark", "problem");
    s.setAttribute("data-flink", c.fader);
    s.innerHTML =
      `<div class="strip-head">` +
      `<div class="mute${muted ? "" : " lit"}" data-testid="mute" data-muted="${attr}" data-binding="ready" aria-disabled="false" style="--tc:#FFA529;--tt:#10101a;">` +
      `<span class="strip-label" data-testid="strip-label" style="--n:${[...c.label].length};">${c.label}</span>` +
      `<span class="strip-marks${l.crowd ? " crowd" : ""}" data-testid="strip-marks">${flag}${chain}${fader}<span class="mute-mark" data-testid="mute-mark" aria-hidden="true">MUTED</span></span>` +
      `</div>` +
      `<div class="strip-info"><div class="strip-readout" data-testid="strip-readout"><div class="status"></div>` +
      `<span class="db" data-testid="db">−69.9</span>${word ? `<span class="strip-mark" data-testid="strip-mark">${word}</span>` : ""}</div></div>` +
      `</div>` +
      `<div class="strip-zone" style="--u:0.85;">` +
      `<div class="strip-col" data-testid="strip-col" aria-hidden="true"><div class="lufs-cap" data-testid="lufs-cap" data-shown="false"><span></span><span></span></div><div class="lufs" data-testid="lufs" data-l="none"></div>` +
      `${c.group ? `<span class="group-side" data-testid="group-side">${c.group}</span>` : ""}</div>` +
      `<div class="scale" data-testid="scale" aria-hidden="true">${scale}</div>` +
      `<div class="fader" data-testid="fader" style="--p:0;"><div class="fader-track" data-testid="meter">${bar}${bar}</div>` +
      `<div class="fader-ghost"></div><div class="fader-rail"><div class="fader-cap" data-testid="fader-cap"></div></div></div>` +
      `<button type="button" class="strip-sel" data-testid="detail-key" aria-label="Channel detail (hold)" data-holding="false" data-hint="false">` +
      `<span class="strip-sel-key" aria-hidden="true"><span class="strip-sel-word">SEL</span></span></button>` +
      `</div>`;
    wrap.appendChild(s);
    host.appendChild(wrap);
    return s;
  });
  // The bundled faces, loaded after a forced layout (`fonts.ready` read
  // right after the markup can resolve before a load starts).
  void host.offsetHeight;
  await Promise.all([...document.fonts].map((f) => f.load().catch(() => f)));
  const faces = [...document.fonts].filter((f) => f.status !== "loaded").map((f) => `${f.family} ${f.weight} ${f.status}`);

  /** An element's box and whether it can be seen: drawn, and inside every box that clips it up to the strip. */
  const seenBox = (el: Element | null, strip: Element) => {
    if (!el) return null;
    const b = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    let seen = cs.display !== "none" && cs.visibility === "visible" && b.width > 0 && b.height > 0;
    for (let a = el.parentElement; a && a !== strip.parentElement; a = a.parentElement) {
      if (getComputedStyle(a).overflow === "visible") continue;
      const r = a.getBoundingClientRect();
      if (b.left < r.left - 0.5 || b.right > r.right + 0.5 || b.top < r.top - 0.5 || b.bottom > r.bottom + 0.5) seen = false;
    }
    return { left: b.left, right: b.right, top: b.top, bottom: b.bottom, width: b.width, seen, display: cs.display };
  };
  /** A text whose glyphs pass its box. */
  const clips = (el: Element) => {
    const box = el as HTMLElement;
    const range = document.createRange();
    range.selectNodeContents(box);
    const t = range.getBoundingClientRect();
    const b = box.getBoundingClientRect();
    return box.scrollWidth > box.clientWidth || t.left < b.left - 0.5 || t.right > b.right + 0.5;
  };
  const problems: string[] = [];
  const yielded: Record<string, number> = {};
  const measures: Result["measures"] = {};
  const at = (n: number) => n.toFixed(2);
  list.forEach((c, i) => {
    const s = strips[i];
    const narrow = c.w < 78;
    const muted = c.link ? c.link.state === "muted" : c.muted;
    const name = `${c.w} px "${c.label}" ${c.link ? `${c.link.state} ${c.link.text}` : c.muted ? "unlinked muted" : "unlinked"}${c.flag ? " !" : ""}${c.fader ? ` ${c.fader}` : ""}${c.conflict ? " conflict" : ""}`;
    const fail = (what: string) => problems.push(`${name}: ${what}`);
    const strip = s.getBoundingClientRect();
    const button = s.querySelector(".strip-head > .mute")!.getBoundingClientRect();
    const info = s.querySelector(".strip-info")!.getBoundingClientRect();
    const line = s.querySelector(".strip-marks")!.getBoundingClientRect();
    const label = s.querySelector(".strip-label") as HTMLElement;
    const lb = label.getBoundingClientRect();
    // The head: the button at the strip's top edge (inside its 1 px
    // border), its whole width, 37 px; 59 px to the readout's foot.
    if (Math.abs(button.height - 37) > 0.5) fail(`the button is ${at(button.height)} px high`);
    if (button.top - strip.top > 1.01) fail(`the button starts ${at(button.top - strip.top)} px under the strip's top`);
    if (Math.abs(button.width - (strip.width - 2)) > 0.5) fail(`the button is ${at(button.width)} px wide in a ${at(strip.width)} px strip`);
    if (Math.abs(info.bottom - button.top - 59) > 0.5) fail(`the head is ${at(info.bottom - button.top)} px`);
    const key = `${c.w}`;
    if (!measures[key]) {
      const sel = s.querySelector(".strip-sel")!.getBoundingClientRect();
      measures[key] = { button: button.height, head: info.bottom - button.top, top: button.top - strip.top, width: button.width, strip: strip.width, key: `${at(sel.width)}x${at(sel.height)}` };
    }
    // The name first, unclipped, at the button's top; the line under it.
    if (clips(label)) fail(`the label is clipped (${label.scrollWidth} > ${label.clientWidth})`);
    if (lb.top - button.top > 4.01) fail(`the label starts ${at(lb.top - button.top)} px under the button's top`);
    if (lb.bottom > line.top + 0.01) fail(`the label's foot ${at(lb.bottom - button.top)} is under the marks line's top ${at(line.top - button.top)}`);
    if (line.bottom > button.bottom + 0.01) fail("the marks line passes the button's foot");
    // Every mark the rule did not let yield is seen whole on the line.
    for (const m of s.querySelectorAll(".strip-marks > *")) {
      const b = seenBox(m, s)!;
      if (b.display === "none") continue;
      if (!b.seen) fail(`.${(m as HTMLElement).className} is not seen whole (${at(b.left - button.left)}..${at(b.right - button.left)}, ${at(b.top - button.top)})`);
    }
    const flag = seenBox(s.querySelector(".strip-flag"), s);
    const chain = seenBox(s.querySelector(".link-mark:not(.flink)"), s);
    const fmark = seenBox(s.querySelector(".link-mark.flink"), s);
    const mmark = s.querySelector(".mute-mark") as HTMLElement;
    const mb = seenBox(mmark, s)!;
    if (flag && !flag.seen) fail("`!` is hidden");
    if (chain && !chain.seen) fail("the chain is hidden");
    const n = [c.flag, !!c.link, !!c.fader, muted].filter(Boolean).length;
    if (fmark) {
      const yields = narrow && c.flag && !!c.link;
      if (fmark.seen === yields) fail(`the fader mark ${fmark.seen ? "shows" : "is hidden"} (${n} marks, narrow ${narrow})`);
      if (yields) yielded[key] = (yielded[key] ?? 0) + 1;
      const db = s.querySelector(".link-mark.flink .link-n")!;
      const dbShown = getComputedStyle(db).display !== "none";
      if (fmark.seen && dbShown !== (n < 3 && !narrow)) fail(`the fader mark's dB ${dbShown ? "shows" : "is hidden"} (${n} marks, narrow ${narrow})`);
    }
    if (c.link) {
      const count = s.querySelector(".link-mark:not(.flink) .link-n")!;
      const countShown = getComputedStyle(count).display !== "none";
      if (countShown === muted) fail(`the chain's count ${countShown ? "shows on a muted strip" : "is hidden on a strip that plays"}`);
    }
    // MUTED on every muted strip, quiet red, 7 px; never on another.
    if (muted) {
      if (!mb.seen) fail("MUTED is not seen");
      if (mmark.textContent !== "MUTED") fail(`the mute word is ${mmark.textContent}`);
      const cs = getComputedStyle(mmark);
      if (cs.color !== "rgb(168, 71, 79)") fail(`MUTED is ${cs.color}`);
      if (cs.fontSize !== "7px") fail(`MUTED is ${cs.fontSize}`);
      if (Math.abs(mb.right - (line.right)) > 0.5) fail("MUTED is not at the line's right end");
    } else if (mb.seen) fail("MUTED shows on a strip that plays");
    // No RET anywhere.
    if (s.querySelector(".strip-ret") || /\bRET\b/.test(s.textContent ?? "")) fail("RET on the strip");
    // The readout's texts fit their boxes.
    for (const t of s.querySelectorAll(".strip-readout .db, .strip-readout .strip-mark")) {
      if (getComputedStyle(t).display !== "none" && clips(t)) fail(`.${(t as HTMLElement).className} "${t.textContent}" is clipped`);
    }
    // The group's name shown whole.
    const side = s.querySelector(".group-side") as HTMLElement | null;
    if (side && (side.scrollWidth > side.clientWidth || side.scrollHeight > side.clientHeight)) fail("the group's name is clipped");
    // SEL: the box the mockup's formula gives, its word unclipped, clear of
    // the cap at −∞ (the static fader's `--p: 0`).
    const zone = s.querySelector(".strip-zone")!.getBoundingClientRect();
    const capW = Math.min(32, c.w - 38);
    // The main session's ruling on #151: 3 px short of the cap's widest reach.
    const want = Math.min(36, zone.width - 24 - capW / 2);
    const sel = s.querySelector(".strip-sel")!.getBoundingClientRect();
    const cap = s.querySelector(".fader-cap")!.getBoundingClientRect();
    if (Math.abs(sel.width - want) > 0.5) fail(`SEL is ${at(sel.width)} px wide, not ${at(want)}`);
    if (Math.abs(sel.height - (narrow ? 44 : 36)) > 0.5) fail(`SEL is ${at(sel.height)} px high`);
    if (Math.abs(sel.left - strip.left - 1) > 0.5) fail(`SEL starts ${at(sel.left - strip.left)} px from the strip's edge`);
    if (sel.right > cap.left - 3 + 0.01) fail(`SEL's right ${at(sel.right - strip.left)} is not 3 px left of the cap's ${at(cap.left - strip.left)}`);
    // Its foot at the strip's (inside its 4 px padding and 1 px border).
    if (Math.abs(strip.bottom - 5 - sel.bottom) > 0.5) fail(`SEL's foot ${at(strip.bottom - sel.bottom)} px above the strip's`);
    if (clips(s.querySelector(".strip-sel-word")!)) fail("SEL is clipped");
    // The info column clear of the cap's widest reach.
    const col = s.querySelector(".strip-col")!.getBoundingClientRect();
    if (col.right > cap.left - 2.99) fail(`the info column ends ${at(cap.left - col.right)} px from the cap`);
    // Nothing passes the strip's right or bottom edge (the fader's rail is
    // a transparent layer as tall as the track, moved by --p; a bar's moving
    // layers are clipped by the bar).
    for (const el of s.querySelectorAll("*")) {
      const r = el.getBoundingClientRect();
      if ((r.width > 0 || r.height > 0) && r.right > strip.right + 0.5) fail(`.${(el as HTMLElement).className} ends at ${at(r.right - strip.left)}, past the strip's ${at(strip.width)}`);
      const clippedLayer = el.classList.contains("fader-rail") || (!!el.closest(".meter-bar") && !el.classList.contains("meter-bar"));
      if ((r.width > 0 || r.height > 0) && r.bottom > strip.bottom + 0.5 && !clippedLayer) fail(`.${(el as HTMLElement).className} ends ${at(r.bottom - strip.bottom)} px under the strip`);
    }
    // The armed guard's and an open write's outline stay inside the button
    // (it is the strip's top edge: an outline outside it would be cut).
    const mute = s.querySelector(".strip-head > .mute") as HTMLElement;
    for (const look of [["armed", null], [null, "unconfirmed"], [null, "not_sent"]] as const) {
      if (look[0]) mute.classList.add(look[0]);
      if (look[1]) mute.setAttribute("data-intent", look[1]);
      const cs = getComputedStyle(mute);
      const reach = parseFloat(cs.outlineOffset) + parseFloat(cs.outlineWidth);
      if (cs.outlineStyle === "none" || reach > 0.01) fail(`the ${look[0] ?? look[1]} outline reaches ${at(reach)} px out of the button`);
      if (look[0]) mute.classList.remove(look[0]);
      if (look[1]) mute.removeAttribute("data-intent");
    }
  });
  host.remove();
  return { faces, problems, yielded, measures };
}

test.describe("The v8 strip head and SEL", () => {
  test("the mockup's combinations fit at 83 and 64 px: `!`, the chain and MUTED never yield, the fader mark only on a narrow strip beside `!` and a chain", async ({ page }) => {
    await openSurface(page);
    const result = await page.evaluate(sweep, combinations());
    expect(result.faces, "the bundled faces loaded").toEqual([]);
    expect(result.problems).toEqual([]);
    // The all-four case yields its fader mark at 64 px, never at 83 px
    // (the table's other fader marks have no `!` beside a chain).
    expect(result.yielded["64"]).toBe(1);
    expect(result.yielded["83.16"]).toBeUndefined();
  });

  test("every link state, `!` and fader mark fits from 64 to 120 px, the head 59 px and SEL clear of the cap", async ({ page }) => {
    await openSurface(page);
    const result = await page.evaluate(sweep, sweepCases());
    expect(result.faces, "the bundled faces loaded").toEqual([]);
    console.log(`v8 head sweep (${test.info().project.name}): ${JSON.stringify(result.measures)} yielded ${JSON.stringify(result.yielded)}`);
    expect(result.problems).toEqual([]);
    // The narrow boundary: 77.99 px is narrow (a 44 px SEL), 78 px is not.
    expect(result.measures["77.99"].key).toMatch(/x44\.00$/);
    expect(result.measures["78"].key).toMatch(/x36\.00$/);
    // The iPad's 83 px strip: 36 px high, 3 px short of the cap.
    expect(result.measures["83.16"].key).toMatch(/^33\.\d\dx36\.00$/);
    expect(result.measures["88"].key).toBe("36.00x36.00");
  });
});
