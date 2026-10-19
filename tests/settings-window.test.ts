import { describe, expect, it } from "vitest";
import { keyError } from "@/settings/_lib/key-error";
import { moveItem, pageFromSearch, searchWithPage } from "@/settings/_lib/pages";
import { isContextMenuKey, menuFocusIndex, menuPosition } from "@/ui/menu-focus";

describe("pageFromSearch", () => {
  it("opens the page named by ?page=", () => {
    expect(pageFromSearch("?page=appearance")).toBe("appearance");
    expect(pageFromSearch("?page=widget&panel=light")).toBe("widget");
    expect(pageFromSearch("?panel=dark&page=opencode")).toBe("opencode");
    expect(pageFromSearch("?page=about")).toBe("about");
  });

  it("still accepts the older ?tab=", () => {
    expect(pageFromSearch("?tab=claude")).toBe("claude");
  });

  it("falls back to General for a missing or unknown page", () => {
    expect(pageFromSearch("")).toBe("general");
    expect(pageFromSearch("?page=nope")).toBe("general");
    expect(pageFromSearch("?page=")).toBe("general");
  });

  it("keeps other parameters when the page changes", () => {
    expect(searchWithPage("?panel=light&tab=codex", "grok")).toBe("?panel=light&page=grok");
    expect(searchWithPage("", "general")).toBe("?page=general");
  });
});

describe("moveItem", () => {
  it("moves one item and leaves the input alone", () => {
    const list = ["a", "b", "c", "d"];
    expect(moveItem(list, 0, 2)).toEqual(["b", "c", "a", "d"]);
    expect(moveItem(list, 3, 0)).toEqual(["d", "a", "b", "c"]);
    expect(moveItem(list, 1, 9)).toEqual(list);
    expect(list).toEqual(["a", "b", "c", "d"]);
  });
});

describe("menuFocusIndex", () => {
  it("wraps with the arrow keys", () => {
    expect(menuFocusIndex("ArrowDown", 0, 3)).toBe(1);
    expect(menuFocusIndex("ArrowDown", 2, 3)).toBe(0);
    expect(menuFocusIndex("ArrowUp", 0, 3)).toBe(2);
    expect(menuFocusIndex("ArrowUp", 2, 3)).toBe(1);
  });

  it("starts at the first or last item when nothing is focused", () => {
    expect(menuFocusIndex("ArrowDown", -1, 4)).toBe(0);
    expect(menuFocusIndex("ArrowUp", -1, 4)).toBe(3);
  });

  it("jumps with Home and End", () => {
    expect(menuFocusIndex("Home", 2, 4)).toBe(0);
    expect(menuFocusIndex("End", 0, 4)).toBe(3);
  });

  it("ignores other keys and empty menus", () => {
    expect(menuFocusIndex("a", 0, 3)).toBeNull();
    expect(menuFocusIndex("Tab", 0, 3)).toBeNull();
    expect(menuFocusIndex("ArrowDown", 0, 0)).toBeNull();
  });
});

describe("isContextMenuKey", () => {
  it("knows the Menu key and Shift+F10", () => {
    expect(isContextMenuKey({ key: "ContextMenu", shiftKey: false })).toBe(true);
    expect(isContextMenuKey({ key: "F10", shiftKey: true })).toBe(true);
    expect(isContextMenuKey({ key: "F10", shiftKey: false })).toBe(false);
    expect(isContextMenuKey({ key: "Enter", shiftKey: true })).toBe(false);
  });
});

describe("menuPosition", () => {
  const viewport = { width: 800, height: 600 };
  const size = { width: 200, height: 160 };
  it("opens below and right of the point", () => {
    expect(menuPosition({ x: 100, y: 100 }, size, viewport)).toEqual({ left: 100, top: 100 });
  });
  it("flips near the right and bottom edges", () => {
    expect(menuPosition({ x: 750, y: 580 }, size, viewport)).toEqual({ left: 550, top: 420 });
  });
  it("never leaves the viewport", () => {
    expect(menuPosition({ x: 100, y: 100 }, size, { width: 150, height: 120 })).toEqual({
      left: 8,
      top: 8,
    });
  });
});

describe("keyError", () => {
  it("accepts both key formats and an empty field", () => {
    expect(keyError("")).toBeNull();
    expect(keyError("oc_sk_0123456789ab_abcdefghijklmnopqrstuvwxyz012345")).toBeNull();
    expect(keyError("  sk-legacy-key  ")).toBeNull();
  });
  it("names what is wrong", () => {
    expect(keyError("pk-123")).toMatch(/start with oc_sk_/);
    expect(keyError("oc_sk_")).toMatch(/incomplete/);
    expect(keyError("oc_sk_ab cd")).toMatch(/spaces/);
    expect(keyError(`sk-${"x".repeat(600)}`)).toMatch(/too long/);
  });
});
