import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { mockApi, providers, setupWidget } from "./fixtures";

test.use({ viewport: { width: 238, height: 700 } });

async function openWidget(page: Page, overrides: Parameters<typeof mockApi>[1] = {}) {
  await setupWidget(page);
  const seen = await mockApi(page, overrides);
  await page.goto("/widget");
  await expect(page.getByRole("region", { name: /^Cursor/ }).first()).toBeVisible();
  return seen;
}

const expand = (page: Page) =>
  page.evaluate(() => (window as unknown as { __emit: (n: string) => void }).__emit("expand"));

test("shows the top account and every enabled provider, with the secret on each call", async ({
  page,
}) => {
  const seen = await openWidget(page);
  const main = page.getByRole("region", { name: "OpenCode account Main" });
  await expect(main).toBeVisible();
  await expect(main.getByRole("meter", { name: "Monthly usage" })).toHaveAttribute(
    "aria-valuenow",
    "85",
  );
  await expect(page.getByRole("region", { name: /^OpenCode account/ })).toHaveCount(1);
  for (const name of [/^Cursor/, /^Grok Bot/, /^Codex/, /^Claude/]) {
    await expect(page.getByRole("region", { name })).toBeVisible();
  }
  await expect(main.getByText("Peak starts in 3m")).toBeVisible();
  expect(seen.every((r) => r.secret === "test-secret")).toBe(true);
});

test("providers render in the saved order", async ({ page }) => {
  await openWidget(page, {
    settings: { providerOrder: ["claude", "cursor", "grok", "codex", "opencode"] },
  });
  const labels = await page
    .getByRole("region")
    .evaluateAll((els) => els.map((el) => el.getAttribute("aria-label") ?? ""));
  expect(labels[0]).toMatch(/^Claude/);
  expect(labels.findIndex((label) => label.startsWith("OpenCode"))).toBeGreaterThan(0);
});

test("right-click menu has icon actions and a red Quit action", async ({ page }) => {
  await openWidget(page);
  const main = page.getByRole("main", { name: "Provider usage" });
  const mainBox = await main.boundingBox();
  await main.click({ button: "right", position: { x: 180, y: 80 } });
  const menu = page.getByRole("menu", { name: "Widget menu" });
  const menuBox = await menu.boundingBox();
  expect(menuBox?.width).toBe(120);
  expect(menuBox?.x).toBeGreaterThanOrEqual(mainBox?.x ?? 0);
  expect((menuBox?.x ?? 0) + (menuBox?.width ?? 0)).toBeLessThanOrEqual(
    (mainBox?.x ?? 0) + (mainBox?.width ?? 0),
  );
  expect(menuBox?.y).toBeGreaterThanOrEqual(mainBox?.y ?? 0);
  await expect(menu.getByRole("menuitem", { name: "Expand" })).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Refresh" })).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Settings" })).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Quit" })).toBeVisible();
  await expect(menu.getByText("Open log folder")).toHaveCount(0);
  await expect(menu.getByRole("menuitem", { name: "Quit" })).toHaveCSS(
    "color",
    "rgb(255, 100, 103)",
  );
});

test("right-click menu omits expand when there is nothing to expand", async ({ page }) => {
  await openWidget(page, { settings: { opencodeEnabled: false } });
  const main = page.getByRole("main", { name: "Provider usage" });
  await main.click({ button: "right", position: { x: 180, y: 80 } });
  const menu = page.getByRole("menu", { name: "Widget menu" });
  await expect(menu.getByRole("menuitem", { name: "Expand" })).toHaveCount(0);
  await expect(menu.getByRole("menuitem", { name: "Refresh" })).toBeVisible();
});

test("the widget shows only the account a provider is signed into", async ({ page }) => {
  const current = providers.cursor.accounts[0];
  await openWidget(page, {
    providers: {
      cursor: {
        accounts: [
          current,
          {
            ...current,
            id: "other@example.com",
            label: "other@example.com",
            active: false,
            stale: true,
            usage: current.usage ? { ...current.usage, accountName: "other@example.com" } : null,
          },
        ],
      },
    },
  });
  await expect(page.getByRole("region", { name: "Cursor dev@example.com" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Cursor other@example.com" })).toHaveCount(0);
});

test("hiding the signed-in account removes it even when others exist", async ({ page }) => {
  const current = providers.cursor.accounts[0];
  await setupWidget(page);
  await mockApi(page, {
    settings: { disabledAccounts: ["cursor:dev@example.com"] },
    providers: {
      cursor: {
        accounts: [
          current,
          {
            ...current,
            id: "other@example.com",
            label: "other@example.com",
            active: false,
            usage: current.usage ? { ...current.usage, accountName: "other@example.com" } : null,
          },
        ],
      },
    },
  });
  await page.goto("/widget");
  await expect(page.getByRole("region", { name: /^Claude/ })).toBeVisible();
  await expect(page.getByRole("region", { name: /^Cursor/ })).toHaveCount(0);
});

test("hiding a provider removes every one of its accounts", async ({ page }) => {
  const current = providers.cursor.accounts[0];
  await setupWidget(page);
  await mockApi(page, {
    settings: { cursorEnabled: false },
    providers: {
      cursor: {
        accounts: [
          current,
          { ...current, id: "other@example.com", label: "other@example.com", active: false },
        ],
      },
    },
  });
  await page.goto("/widget");
  await expect(page.getByRole("region", { name: /^Claude/ })).toBeVisible();
  await expect(page.getByRole("region", { name: /^Cursor/ })).toHaveCount(0);
});

test("hiding OpenCode removes its cards and leaves the other providers", async ({ page }) => {
  await openWidget(page, { settings: { opencodeEnabled: false } });
  await expect(page.getByRole("region", { name: /^OpenCode account/ })).toHaveCount(0);
  await expect(page.getByRole("region", { name: /^Claude/ })).toBeVisible();
});

test("cards never touch: collapsed and expanded both keep a gap", async ({ page }) => {
  await openWidget(page);
  for (const state of ["collapsed", "expanded"]) {
    if (state === "expanded") await expand(page);
    const boxes = await page
      .getByRole("region")
      .evaluateAll((els) =>
        els.map((el) => el.getBoundingClientRect()).map((r) => ({ top: r.top, bottom: r.bottom })),
      );
    for (let i = 1; i < boxes.length; i++) {
      expect(boxes[i].top - boxes[i - 1].bottom, `${state} gap ${i}`).toBeGreaterThanOrEqual(3);
    }
  }
});

test("expanded view hides disabled and lapsed accounts, exhausted ones sink", async ({ page }) => {
  await openWidget(page);
  await expand(page);
  const names = await page
    .getByRole("region", { name: /^OpenCode account/ })
    .evaluateAll((els) => els.map((el) => el.getAttribute("aria-label")));
  expect(names).toEqual(["OpenCode account Main", "OpenCode account Spare"]);
});

test("re-auth and exhausted states are shown without cache labels", async ({ page }) => {
  await openWidget(page);
  await expect(page.getByRole("region", { name: /^Grok Bot/ }).getByText("cached")).toHaveCount(0);
  await expect(page.getByRole("region", { name: /^Claude/ }).getByText("re-auth")).toBeVisible();
  const codex = page.getByRole("region", { name: /^Codex/ });
  const ring = await codex.evaluate((el) => getComputedStyle(el).borderTopColor);
  expect(ring).toBe("rgb(255, 100, 103)"); // ringDanger (dark)
});

test("one failing provider does not blank the others", async ({ page }) => {
  await openWidget(page, { fail: ["cursor"] });
  await expect(
    page.getByRole("region", { name: /^Cursor/ }).getByText("Not signed in or unavailable"),
  ).toBeVisible();
  await expect(
    page.getByRole("region", { name: /^Claude/ }).getByRole("meter", { name: "Weekly usage" }),
  ).toBeVisible();
  await expect(page.getByRole("status")).toHaveCount(0);
});

test("an unreachable server is reported, with a working retry", async ({ page }) => {
  await setupWidget(page);
  await page.route("**/api/**", (route) => route.abort());
  await page.goto("/widget");
  await expect(page.getByRole("status")).toContainText("Server unreachable");
  await page.unrouteAll();
  await mockApi(page);
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(page.getByRole("status")).toHaveCount(0);
  await expect(page.getByRole("region", { name: "OpenCode account Main" })).toBeVisible();
});

test("copy-key button copies the key", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const seen = await openWidget(page);
  await page.getByRole("button", { name: "Copy API key for Main" }).click();
  await expect(page.getByText("Key copied")).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("sk-test-key-123");
  expect(seen.some((r) => r.path === "/api/key?email=main%40example.com")).toBe(true);
});

for (const themeMode of ["dark", "light"] as const) {
  test(`${themeMode} theme passes axe (WCAG 2.2 AA) and matches the snapshot`, async ({ page }) => {
    await openWidget(page, { settings: { themeMode } });
    await expand(page);
    const results = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
      .analyze();
    expect(
      results.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target).join(" ")}`),
    ).toEqual([]);
    await expect(page.locator("main")).toHaveScreenshot(`widget-${themeMode}.png`, {
      animations: "disabled",
    });
  });
}
