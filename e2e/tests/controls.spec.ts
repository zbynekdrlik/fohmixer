import { test, expect } from "./support/fixtures";
import { LiveClient, centre, hostLine, openSurface, ready, strip, track, until } from "./support/live";

// Solo, the stage mics with STAGE AUT, TechAlert and REFRESH ALL (spec F6,
// F7, F14–F16, I5).

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
    await expect(solo).toHaveCSS("background-color", "rgb(60, 60, 60)");
    await solo.click();
    await until(() => live.get("band", stems, "solo"), (v) => v === true, "Stems soloed");
    await expect(solo).toHaveAttribute("data-on", "true");
    await expect(solo).toHaveCSS("background-color", "rgb(61, 97, 184)");
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
      const seen = new Set<string>();
      const deadline = Date.now() + 700;
      while (Date.now() < deadline) {
        seen.add((await overlay.getAttribute("data-visible")) ?? "");
        await page.waitForTimeout(40);
      }
      expect([...seen].sort()).toEqual(["false", "true"]);
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
    // Its strip's mute is the same track's.
    await expect(strip(page, "TechAlert #").getByTestId("mute")).toHaveAttribute("data-muted", "true");
  });
});

test.describe("REFRESH ALL", () => {
  test("a renamed track turns its strip red; renamed back, the hub binds it again by itself", async ({ page }) => {
    await openSurface(page);
    const hand2 = strip(page, "Hand2 #");
    const surface = page.getByTestId("surface");
    const refreshes = await surface.getAttribute("data-refreshes");
    await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "bound");
    try {
      expect(await hostLine("band", 'rename "Hand2 #" "Hand2 X"')).toBe("RENAMED 1");
      await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "unbound");
      await expect(hand2.getByTestId("fader")).toHaveAttribute("data-binding", "unresolved");
    } finally {
      await hostLine("band", 'rename "Hand2 X" "Hand2 #"');
    }
    // The hub's name guard (F3) heals the live subscription: no refresh.
    await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "bound");
    await ready(hand2.getByTestId("fader"));
    await expect(surface).toHaveAttribute("data-refreshes", refreshes ?? "");
  });

  test("a subscription made while the name is gone stays red until REFRESH ALL", async ({ page }) => {
    await openSurface(page);
    const hand2 = strip(page, "Hand2 #");
    const refresh = page.getByTestId("refresh");
    const surface = page.getByTestId("surface");
    await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "bound");
    try {
      expect(await hostLine("band", 'rename "Hand2 #" "Hand2 X"')).toBe("RENAMED 1");
      await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "unbound");
      // A refresh now subscribes a name that is gone: it cannot bind.
      const before = Number(await surface.getAttribute("data-refreshes"));
      await refresh.click();
      await expect(refresh).toHaveAttribute("data-flash", "true");
      await expect(refresh).toHaveAttribute("data-flash", "false");
      await expect(surface).toHaveAttribute("data-refreshes", String(before + 1));
      await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "unbound");
      await expect(hand2.getByTestId("fader")).toHaveAttribute("aria-disabled", "true");
    } finally {
      await hostLine("band", 'rename "Hand2 X" "Hand2 #"');
    }
    // Renamed back, that failed subscription stays red (nothing to heal)…
    await page.waitForTimeout(1000);
    await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "unbound");
    // …until REFRESH ALL resolves every name afresh (F7).
    await refresh.click();
    await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "bound");
    await ready(hand2.getByTestId("fader"));
  });

  test("refresh unfolds the configured group tracks", async ({ page }) => {
    const group = track("Vocals Repro grp#");
    await live.set("band", group, "fold_state", true);
    expect(await live.get("band", group, "fold_state")).toBe(1);
    await openSurface(page);
    // The automatic refresh after the load already unfolded it.
    await until(() => live.get("band", group, "fold_state"), (v) => v === 0, "unfolded by the automatic refresh");
    await live.set("band", group, "fold_state", true);
    await page.getByTestId("refresh").click();
    await until(() => live.get("band", group, "fold_state"), (v) => v === 0, "unfolded by REFRESH ALL");
  });
});
