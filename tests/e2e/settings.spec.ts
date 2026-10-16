import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { mockApi, providers, setupWidget } from "./fixtures";

test.use({ viewport: { width: 1000, height: 760 } });

async function openSettings(page: Page, overrides: Parameters<typeof mockApi>[1] = {}) {
  await setupWidget(page);
  const seen = await mockApi(page, overrides);
  await page.goto("/settings");
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("main@example.com")).toBeVisible();
  return { seen, dialog };
}

test("tabs follow the WAI-ARIA pattern", async ({ page }) => {
  const { dialog } = await openSettings(page);
  const accountsTab = dialog.getByRole("tab", { name: "Accounts" });
  await expect(accountsTab).toHaveAttribute("aria-selected", "true");
  await accountsTab.focus();
  await page.keyboard.press("ArrowLeft");
  await expect(dialog.getByRole("tab", { name: "General" })).toBeFocused();
  await expect(dialog.getByRole("tabpanel")).toContainText("Panel theme");
});

test("switches expose state and send only the changed field", async ({ page }) => {
  const { seen, dialog } = await openSettings(page);
  await dialog.getByRole("tab", { name: "General" }).click();
  const sw = dialog.getByRole("switch", { name: "Show provider names" });
  await expect(sw).toHaveAttribute("aria-checked", "false");
  await sw.click();
  await expect(sw).toHaveAttribute("aria-checked", "true");
  const post = seen.find((r) => r.method === "POST" && r.path === "/api/settings");
  expect(post?.body).toEqual({ showProviderNames: true });
});

test("theme choices are a radio group", async ({ page }) => {
  const { seen, dialog } = await openSettings(page);
  await dialog.getByRole("tab", { name: "General" }).click();
  const group = dialog.getByRole("radiogroup", { name: "Widget theme" });
  await group.getByRole("radio", { name: "light" }).click();
  await expect(group.getByRole("radio", { name: "light" })).toHaveAttribute("aria-checked", "true");
  await expect
    .poll(() =>
      seen.some((r) => r.method === "POST" && JSON.stringify(r.body) === '{"themeMode":"light"}'),
    )
    .toBe(true);
});

test("account details show a key hint and never fetch the full key", async ({ page }) => {
  const { seen, dialog } = await openSettings(page);
  await dialog.getByRole("button", { name: "Details for main@example.com" }).click();
  const detail = page.getByRole("dialog", { name: "OpenCode" });
  await expect(detail.getByText("sk-…ab12")).toBeVisible();
  expect(seen.some((r) => r.path.startsWith("/api/key"))).toBe(false);
});

test("Escape closes only the topmost dialog", async ({ page }) => {
  const { dialog } = await openSettings(page);
  await dialog.getByRole("button", { name: "Details for main@example.com" }).click();
  await expect(page.getByRole("dialog", { name: "OpenCode" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "OpenCode" })).toHaveCount(0);
  await expect(dialog).toBeVisible();
  expect(
    await page.evaluate(() => (window as unknown as { __closed?: boolean }).__closed),
  ).toBeUndefined();
});

test("nickname saves once on Enter, not on every blur", async ({ page }) => {
  const { seen, dialog } = await openSettings(page);
  await dialog.getByRole("button", { name: "Details for main@example.com" }).click();
  const detail = page.getByRole("dialog", { name: "OpenCode" });
  const field = detail.getByLabel("Nickname");
  await field.focus();
  await field.blur(); // unchanged: no request
  await field.fill("Primary");
  await field.press("Enter");
  await expect(detail.getByText("✓ saved")).toBeVisible();
  const posts = seen.filter((r) => r.method === "POST" && r.path === "/api/settings");
  expect(posts.map((p) => p.body)).toEqual([{ displayNames: { "main@example.com": "Primary" } }]);
});

test("dragging a provider tab left moves it earlier in the order", async ({ page }) => {
  const { seen, dialog } = await openSettings(page);
  const providers = dialog.getByRole("tablist", { name: "Providers" });
  await expect(dialog.getByText("Drag up or down to reorder on the widget")).toBeVisible();
  const claude = providers.getByRole("tab", { name: "Claude" });
  const opencode = providers.getByRole("tab", { name: "OpenCode" });
  const from = await claude.boundingBox();
  const to = await opencode.boundingBox();
  expect(from).not.toBeNull();
  expect(to).not.toBeNull();
  await page.mouse.move(from!.x + from!.width / 2, from!.y + from!.height / 2);
  await page.mouse.down();
  await page.mouse.move(to!.x + to!.width / 2, to!.y + to!.height / 2, { steps: 5 });
  await expect(providers.locator("[data-reorder-preview]")).toHaveCount(1);
  await page.mouse.up();
  expect(seen.find((r) => r.method === "POST" && r.path === "/api/settings")?.body).toEqual({
    providerOrder: ["claude", "opencode", "cursor", "grok", "codex"],
  });
  await expect(providers.getByRole("tab").first()).toContainText("Claude");
});

test("a Cursor account can be nicknamed and hidden on its own", async ({ page }) => {
  const current = providers.cursor.accounts[0];
  const { seen, dialog } = await openSettings(page, {
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
          },
        ],
      },
    },
  });
  await dialog.getByRole("tab", { name: "Cursor" }).click();
  await expect(dialog.getByText("dev@example.com")).toBeVisible();
  await expect(dialog.getByText("other@example.com")).toBeVisible();
  await expect(dialog.getByText("in use")).toHaveCount(0);
  await dialog.getByRole("button", { name: "Details for other@example.com" }).click();
  const detail = page.getByRole("dialog", { name: "Cursor" });
  await detail.getByRole("switch", { name: "Show this account" }).click();
  await detail.getByLabel("Nickname").fill("Alt");
  await detail.getByLabel("Nickname").press("Enter");
  await expect(detail.getByText("✓ saved")).toBeVisible();
  const posts = seen.filter((r) => r.method === "POST" && r.path === "/api/settings");
  expect(posts.map((p) => p.body)).toEqual([
    { disabledAccounts: ["hidden@example.com", "cursor:other@example.com"] },
    { displayNames: { "cursor:other@example.com": "Alt" } },
  ]);
});

test("OpenCode can be hidden from the widget as a whole", async ({ page }) => {
  const { seen, dialog } = await openSettings(page);
  const sw = dialog.getByRole("switch", { name: "Show on widget" });
  await expect(sw).toHaveAttribute("aria-checked", "true");
  await sw.click();
  await expect(sw).toHaveAttribute("aria-checked", "false");
  expect(seen.find((r) => r.method === "POST" && r.path === "/api/settings")?.body).toEqual({
    opencodeEnabled: false,
  });
});

test("adding an account validates the key and posts it", async ({ page }) => {
  const { seen, dialog } = await openSettings(page);
  await dialog.getByRole("button", { name: "Add", exact: true }).click();
  const add = page.getByRole("dialog", { name: "Add OpenCode account" });
  await add.getByLabel("Email or label").fill("new@example.com");
  await add.getByLabel("API key").fill("not-a-key");
  await add.getByRole("button", { name: "Add account" }).click();
  expect(seen.some((r) => r.method === "POST" && r.path === "/api/accounts")).toBe(false);
  await add.getByLabel("API key").fill("sk-new-key");
  await add.getByRole("button", { name: "Add account" }).click();
  await expect(page.getByText("Account added")).toBeVisible();
  expect(seen.find((r) => r.method === "POST" && r.path === "/api/accounts")?.body).toEqual({
    email: "new@example.com",
    key: "sk-new-key",
  });
});

test("removing an account asks for confirmation", async ({ page }) => {
  const { seen, dialog } = await openSettings(page);
  await dialog.getByRole("button", { name: "Remove spare@example.com" }).click();
  expect(seen.some((r) => r.method === "DELETE")).toBe(false);
  await dialog.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(page.getByText("Account removed")).toBeVisible();
  expect(seen.find((r) => r.method === "DELETE")?.body).toEqual({ email: "spare@example.com" });
});

for (const panelTheme of ["dark", "light"] as const) {
  test(`${panelTheme} panel passes axe (WCAG 2.2 AA) on every tab`, async ({ page }) => {
    const { dialog } = await openSettings(page, { settings: { panelTheme } });
    const axe = () =>
      new AxeBuilder({ page })
        .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
        .analyze();
    const check = async (label: string) => {
      const { violations } = await axe();
      expect(
        violations.map((v) => `${label} ${v.id}: ${v.nodes.map((n) => n.target).join(" ")}`),
      ).toEqual([]);
    };
    await check("accounts/opencode");
    await expect(dialog).toHaveScreenshot(`settings-accounts-${panelTheme}.png`, {
      animations: "disabled",
    });
    for (const tab of ["Cursor", "Grok Bot", "Codex", "Claude"]) {
      await dialog.getByRole("tab", { name: tab }).click();
      await check(`accounts/${tab}`);
    }
    await dialog.getByRole("tab", { name: "General" }).click();
    await check("general");
    await expect(dialog).toHaveScreenshot(`settings-${panelTheme}.png`, { animations: "disabled" });
  });
}
