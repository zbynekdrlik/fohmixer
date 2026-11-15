// What sectors.spec.ts and sector-detail.spec.ts share (#148, #205, #208):
// the documents placing the e2e fixture's frame strips, the sectors page's
// locators and reads, a second page's console, the hub's `sectors` records.
import type { Page } from "@playwright/test";
import { expect } from "./fixtures";
import { hubEvents, openSurface, type SectorsDoc, type StripRef } from "./live";

export const band = (label: string): StripRef => ({ instance: "band", label });
export const master = (label: string): StripRef => ({ instance: "master", label });

export const VOCAL = band("Vocal 1 repro#");
export const KEYS = band("Keys 1");

/** Every sector placed; lu and ld with two layouts each. */
export const DOC: SectorsDoc = {
  rev: 0,
  sectors: {
    lu: [
      { id: "lu-1", title: "STAGE", strips: [VOCAL, KEYS] },
      { id: "lu-2", title: "ALT", strips: [band("Hand2 #")] },
    ],
    ru: [{ id: "ru-1", title: "MAIN", strips: [band("B-Main repro #")] }],
    ld: [
      { id: "ld-1", title: "HANDS", strips: [master("Hand1 #"), master("Hand4 #")] },
      { id: "ld-2", title: "FX", strips: [band("A-Reverb #")] },
    ],
    rd: [{ id: "rd-1", title: "MAIN", strips: [master("A-Echo")] }],
  },
};

/** lu's STAGE at exactly 6 units (full: one more strip is refused) and master Hand2 # on the page in ld. */
export const FULL_DOC: SectorsDoc = {
  rev: 0,
  sectors: {
    lu: [
      {
        id: "lu-1",
        title: "STAGE",
        strips: [VOCAL, KEYS, band("A-Reverb #"), master("A-Reverb #"), master("Hand1 #"), master("Hand4 #")],
      },
    ],
    ld: [{ id: "ld-1", title: "HANDS", strips: [master("Hand2 #")] }],
  },
};

/** The sectors page, as CSS (for a read in one task inside the page, #198). */
export const SECTORS = '[data-testid="page"][data-sectors="true"]';
export const sectorsPage = (page: Page) => page.locator(SECTORS);

/** A sector's slots on the sectors page, in their order: as CSS, and as a locator. */
export const slotsCss = (sector: string) => `${SECTORS} .slot[data-sector="${sector}"]`;
export const slots = (page: Page, sector: string) => page.locator(slotsCss(sector));

/** A sector's strips as `<instance>|<track>`, in their order. */
export async function stripsOf(page: Page, sector: string): Promise<string[]> {
  return slots(page, sector)
    .locator('[data-testid="strip"]')
    .evaluateAll((all) => all.map((s) => `${s.getAttribute("data-instance")}|${s.getAttribute("data-track")}`));
}

export const name = (r: StripRef) => `${r.instance}|${r.label}`;

/** Opens the surface and waits for the sectors page. */
export async function openSectors(page: Page) {
  await openSurface(page);
  await expect(sectorsPage(page)).toBeVisible();
}

/** The page's `--strip-w`. */
export function stripWidth(page: Page): Promise<string> {
  return sectorsPage(page).evaluate((e) => (e as HTMLElement).style.getPropertyValue("--strip-w"));
}

/** A second page's console (the fixture's guard watches the test's own page only). */
export function watchConsole(page: Page): string[] {
  const problems: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") problems.push(`[${msg.type()}] ${msg.text()}`);
  });
  page.on("pageerror", (error) => problems.push(`[pageerror] ${error.message}`));
  return problems;
}

/** The hub's `sectors` records since `since` (the runner's clock), each `what`. */
export async function sectorSteps(since: number): Promise<string[]> {
  return (await hubEvents()).filter((r: any) => r.ev === "sectors" && r.ts >= since).map((r: any) => r.what);
}
