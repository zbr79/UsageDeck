import { describe, expect, it } from "vitest";
import { menuNavIndex } from "@/widget/_lib/menu-nav";

describe("menuNavIndex", () => {
  it("moves down and wraps from the last item to the first", () => {
    expect(menuNavIndex("ArrowDown", 0, 4)).toBe(1);
    expect(menuNavIndex("ArrowDown", 3, 4)).toBe(0);
  });

  it("moves up and wraps from the first item to the last", () => {
    expect(menuNavIndex("ArrowUp", 2, 4)).toBe(1);
    expect(menuNavIndex("ArrowUp", 0, 4)).toBe(3);
  });

  it("jumps to the ends with Home and End", () => {
    expect(menuNavIndex("Home", 2, 4)).toBe(0);
    expect(menuNavIndex("End", 0, 4)).toBe(3);
  });

  it("enters the menu from outside: Down to the first item, Up to the last", () => {
    expect(menuNavIndex("ArrowDown", -1, 3)).toBe(0);
    expect(menuNavIndex("ArrowUp", -1, 3)).toBe(2);
  });

  it("ignores other keys and empty menus", () => {
    expect(menuNavIndex("Enter", 1, 4)).toBeNull();
    expect(menuNavIndex("a", 1, 4)).toBeNull();
    expect(menuNavIndex("ArrowDown", 0, 0)).toBeNull();
  });
});
