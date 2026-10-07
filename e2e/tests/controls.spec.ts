import { test, expect } from "./support/fixtures";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  LiveClient,
  centre,
  doubleTap,
  harness,
  hostLine,
  hubSubscriptions,
  openSurface,
  ready,
  selectPage,
  strip,
  track,
  until,
} from "./support/live";

// Solo, the stage mics with STAGE AUT, TechAlert, bindings that stay current
// with no REFRESH ALL and the page's unfold (spec F3, F7, F14–F16, I5; #58).

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
});
test.afterEach(async () => {
  live.close();
});

test.describe("Solo buttons", () => {
  test("a solo toggles its group's solo, independently of the others", async ({ page }) => {
    const stems = track("Stems grp#");
    const vocals = track("Vocals Repro grp#");
    await live.set("band", stems, "solo", false);
    await live.set("band", vocals, "solo", true);
    await openSurface(page);
    const solo = page.locator('[data-testid="solo"][data-track="Stems grp#"]');
    await ready(solo);
    await expect(solo).toHaveText("SOLO Stems");
    await expect(solo).toHaveAttribute("data-on", "false");
    await expect(solo).not.toHaveClass(/\bon\b/);
    await solo.click();
    await until(() => live.get("band", stems, "solo"), (v) => v === true, "Stems soloed");
    await expect(solo).toHaveAttribute("data-on", "true");
    await expect(solo).toHaveClass(/\bon\b/);
    // Independent: the other solo stays as it was.
    expect(await live.get("band", vocals, "solo")).toBe(true);
    await solo.click();
    await until(() => live.get("band", stems, "solo"), (v) => v === false, "Stems unsoloed");
    await live.set("band", vocals, "solo", false);
  });

  test("a solo whose group track is renamed turns red and takes no tap (I5)", async ({ page }) => {
    const stems = track("Stems grp#");
    await live.set("band", stems, "solo", false);
    await openSurface(page);
    const solo = page.locator('[data-testid="solo"][data-track="Stems grp#"]');
    await ready(solo);
    await expect(solo).toHaveAttribute("data-binding", "ready");
    try {
      expect(await hostLine("band", 'rename "Stems grp#" "Stems grp X"')).toBe("RENAMED 1");
      await expect(solo).toHaveAttribute("data-binding", "unresolved");
      await expect(solo).toHaveAttribute("aria-disabled", "true");
      await expect(solo).toHaveCSS("background-color", "rgb(255, 59, 48)");
      const { x, y } = await centre(solo);
      await page.mouse.click(x, y);
      await page.waitForTimeout(300);
      expect(await live.get("band", track("Stems grp X"), "solo")).toBe(false);
    } finally {
      await hostLine("band", 'rename "Stems grp X" "Stems grp#"');
    }
    await expect(solo).toHaveAttribute("data-binding", "ready");
    await ready(solo);
  });
});

test.describe("Stage mics and STAGE AUT", () => {
  test("STAGE AUT mutes the stage mics while Live plays and unmutes them when it stops", async ({ page }) => {
    const mics = track("Mics Stage #");
    await live.call("band", "live_set", "stop_playing");
    await live.set("band", mics, "mute", false);
    await openSurface(page);
    const aut = page.getByTestId("stage-aut");
    const button = page.getByTestId("stage-mics");
    await ready(aut);
    await ready(button);
    await expect(button).toHaveText("STAGE");
    await expect(button).toHaveAttribute("data-muted", "false");
    // Lit while the mics are live, as TouchOSC's button (its script: mute
    // false -> x 1) and every strip's mute (F12, lit when audible): the
    // engineer reads a lit STAGE as "the stage is open" (#9, parity audit #21).
    await expect(button).toHaveClass(/\bon\b/);
    try {
      if ((await aut.getAttribute("data-on")) === "true") await aut.click();
      await expect(aut).toHaveAttribute("data-on", "false");
      await aut.click();
      await expect(aut).toHaveAttribute("data-on", "true");
      await live.call("band", "live_set", "start_playing");
      await until(() => live.get("band", mics, "mute"), (v) => v === true, "muted while playing");
      await expect(button).toHaveAttribute("data-muted", "true");
      await expect(button).not.toHaveClass(/\bon\b/);
      await live.call("band", "live_set", "stop_playing");
      await until(() => live.get("band", mics, "mute"), (v) => v === false, "live when stopped");
      await expect(button).toHaveAttribute("data-muted", "false");
      await expect(button).toHaveClass(/\bon\b/);
    } finally {
      live.setHub("stage_aut", false);
      await live.call("band", "live_set", "stop_playing");
    }
    await expect(aut).toHaveAttribute("data-on", "false");
    // The button itself mutes and unmutes them by hand: dark while muted,
    // lit again once live.
    await button.click();
    await until(() => live.get("band", mics, "mute"), (v) => v === true, "muted by hand");
    await expect(button).not.toHaveClass(/\bon\b/);
    await button.click();
    await until(() => live.get("band", mics, "mute"), (v) => v === false, "live by hand");
    await expect(button).toHaveClass(/\bon\b/);
  });
});

test.describe("TechAlert", () => {
  test("the overlay blinks while the TechAlert track is unmuted", async ({ page }) => {
    const alert = track("TechAlert #");
    await live.set("band", alert, "mute", true);
    await openSurface(page);
    const overlay = page.getByTestId("alert");
    await expect(overlay).toHaveAttribute("data-active", "false");
    await expect(overlay).toHaveAttribute("data-visible", "false");
    try {
      await live.set("band", alert, "mute", false);
      await expect(overlay).toHaveAttribute("data-active", "true");
      // Sampled inside the page every 15 ms for 900 ms (a Playwright read
      // per sample takes long enough in WebKit to alias the 300 ms blink).
      const seen = await overlay.evaluate(async (el) => {
        const states = new Set<string>();
        const end = performance.now() + 900;
        while (performance.now() < end) {
          states.add(el.getAttribute("data-visible") ?? "");
          await new Promise((done) => setTimeout(done, 15));
        }
        return [...states].sort();
      });
      expect(seen).toEqual(["false", "true"]);
      // The overlay never takes a touch.
      await expect(overlay).toHaveCSS("pointer-events", "none");
    } finally {
      await live.set("band", alert, "mute", true);
    }
    await expect(overlay).toHaveAttribute("data-active", "false");
    for (let i = 0; i < 10; i++) {
      await expect(overlay).toHaveAttribute("data-visible", "false");
      await page.waitForTimeout(70);
    }
    // Its button shows the same track's mute: dark while muted.
    await expect(page.getByTestId("alert-toggle")).toHaveAttribute("data-on", "false");
  });

  test("the TechAlert button unmutes and mutes its track; the wash follows", async ({ page }) => {
    const alert = track("TechAlert #");
    await live.set("band", alert, "mute", true);
    await openSurface(page);
    const button = page.getByTestId("rail").getByTestId("alert-toggle");
    const overlay = page.getByTestId("alert");
    await ready(button);
    try {
      await button.click();
      await until(() => live.get("band", alert, "mute"), (v) => v === false, "TechAlert on");
      await expect(button).toHaveAttribute("data-on", "true");
      await expect(overlay).toHaveAttribute("data-active", "true");
      await button.click();
      await until(() => live.get("band", alert, "mute"), (v) => v === true, "TechAlert off");
      await expect(button).toHaveAttribute("data-on", "false");
      await expect(overlay).toHaveAttribute("data-active", "false");
    } finally {
      await live.set("band", alert, "mute", true);
    }
  });
});

test.describe("A guarded TechAlert", () => {
  test("needs a second tap within 500 ms, as a guarded mute", async ({ page }) => {
    // #21 review: a TechAlert strip in double_click_mute keeps its guard.
    const alert = track("TechAlert #");
    await live.set("band", alert, "mute", true);
    const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
    const changed = JSON.parse(readFileSync(LAYOUT, "utf-8"));
    changed.global[0].mute_guard = true;
    await openSurface(page);
    try {
      await harness("/hub/layout", { layout: changed });
      const button = page.getByTestId("rail").getByTestId("alert-toggle");
      await page.waitForTimeout(500);
      await ready(button);
      await button.click();
      await expect(button).toHaveClass(/\barmed\b/);
      await page.waitForTimeout(700);
      expect(await live.get("band", alert, "mute")).toBe(true);
      await expect(button).not.toHaveClass(/\barmed\b/);
      await doubleTap(button, 150);
      await until(() => live.get("band", alert, "mute"), (v) => v === false, "TechAlert on after the confirming tap");
    } finally {
      await live.set("band", alert, "mute", true);
      await harness("/hub/layout/reset");
    }
  });
});

test.describe("SOLO clear", () => {
  test("the pill shows while a solo of the page is on and clears only those", async ({ page }) => {
    // #21: the pill in the top bar turns off every solo Live reports on.
    const stems = track("Stems grp#");
    const vocals = track("Vocals Repro grp#");
    await live.set("band", stems, "solo", false);
    await live.set("band", vocals, "solo", false);
    await openSurface(page);
    const pill = page.getByTestId("solo-clear");
    await ready(page.locator('[data-testid="solo"][data-track="Stems grp#"]'));
    await expect(pill).toHaveAttribute("data-on", "false");
    await expect(pill).toBeHidden();
    try {
      await live.set("band", vocals, "solo", true);
      await expect(pill).toHaveAttribute("data-on", "true");
      await expect(pill).toBeVisible();
      await pill.click();
      await until(() => live.get("band", vocals, "solo"), (v) => v === false, "Vocals unsoloed");
      expect(await live.get("band", stems, "solo")).toBe(false);
      await expect(pill).toBeHidden();
      // Both on: both off. The pill clears the solos the page knows of, and
      // it shows as soon as one is on: wait until the page shows both (a
      // click before Stems' solo reached the page cleared only Vocals, #43
      // PR E's red run).
      await live.set("band", vocals, "solo", true);
      await live.set("band", stems, "solo", true);
      await expect(page.locator('[data-testid="solo"][data-track="Stems grp#"]')).toHaveAttribute("data-on", "true");
      await expect(page.locator('[data-testid="solo"][data-track="Vocals Repro grp#"]')).toHaveAttribute("data-on", "true");
      await expect(pill).toBeVisible();
      await pill.click();
      await until(() => live.get("band", stems, "solo"), (v) => v === false, "Stems unsoloed");
      await until(() => live.get("band", vocals, "solo"), (v) => v === false, "Vocals unsoloed");
      // A page without solos has no pill to show.
      await live.set("band", vocals, "solo", true);
      await page.locator('[data-testid="tab"][data-page="cue"]').click();
      await expect(pill).toBeHidden();
    } finally {
      await live.set("band", stems, "solo", false);
      await live.set("band", vocals, "solo", false);
    }
  });
});

test.describe("Bindings stay current with no REFRESH ALL (#58)", () => {
  test("a renamed track turns its strip red; renamed back, the hub binds it again by itself", async ({ page }) => {
    await openSurface(page);
    const hand2 = strip(page, "Hand2 #");
    await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "bound");
    try {
      expect(await hostLine("band", 'rename "Hand2 #" "Hand2 X"')).toBe("RENAMED 1");
      await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "unbound");
      await expect(hand2.getByTestId("fader")).toHaveAttribute("data-binding", "unresolved");
    } finally {
      await hostLine("band", 'rename "Hand2 X" "Hand2 #"');
    }
    // The hub's name guard (F3) heals the live subscription.
    await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "bound");
    await ready(hand2.getByTestId("fader"));
  });

  test("a subscription made while the name is gone heals by itself when the track is renamed back", async ({ page }) => {
    await openSurface(page);
    const hand2 = strip(page, "Hand2 #");
    await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "bound");
    try {
      expect(await hostLine("band", 'rename "Hand2 #" "Hand2 X"')).toBe("RENAMED 1");
      await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "unbound");
      // Away from the FOH page its strips are unsubscribed; back on it they
      // are subscribed again while the name is gone, so the new
      // subscription has no track whose name the hub could listen to.
      const before = await hubSubscriptions();
      await selectPage(page, "cue");
      await until(() => hubSubscriptions(), (n) => n < before, "the FOH page's subscriptions ended");
      await selectPage(page, "foh");
      await expect(hand2.getByTestId("fader")).toHaveAttribute("data-binding", "unresolved");
      await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "unbound");
    } finally {
      await hostLine("band", 'rename "Hand2 X" "Hand2 #"');
    }
    // Renamed back with no list change: the hub watches every track's name
    // while a binding is in error, so the strip heals with nothing pressed.
    await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "bound");
    await ready(hand2.getByTestId("fader"));
    await expect(page.getByTestId("refresh")).toHaveCount(0);
  });

  test("the page unfolds the configured group tracks once loaded, and again when Live is back", async ({ page }) => {
    // Live sends no meter of a track inside a folded group (#58).
    const group = track("Vocals Repro grp#");
    await live.set("band", group, "fold_state", true);
    expect(await live.get("band", group, "fold_state")).toBe(1);
    await openSurface(page);
    await until(() => live.get("band", group, "fold_state"), (v) => v === 0, "unfolded by the page's load");
    const surface = page.getByTestId("surface");
    await expect(surface).toHaveAttribute("data-unfolds", "1");
    // A Live restart (a set load alike) unfolds the band's groups again.
    await harness("/host/band/restart");
    await expect(surface).toHaveAttribute("data-unfolds", "2", { timeout: 10_000 });
    await until(() => live.get("band", group, "fold_state"), (v) => v === 0, "unfolded once Live is back");
  });
});
