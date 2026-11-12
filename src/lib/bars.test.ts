import { describe, expect, it } from "vitest";
import { shownBars } from "./bars";

describe("shownBars", () => {
  it("every bar by default, in card order", () => {
    expect([...shownBars("opencode", {})]).toEqual(["rolling", "weekly", "monthly"]);
  });

  it("leaves out the hidden bars and keeps the order of the rest", () => {
    expect([...shownBars("opencode", { opencode: ["rolling"] })]).toEqual(["weekly", "monthly"]);
    expect([...shownBars("claude", { claude: ["weekly"] })]).toEqual(["fiveHour", "scoped"]);
    expect([...shownBars("claude", { codex: ["weekly"] })]).toHaveLength(3);
  });

  it("only bars the card has data for", () => {
    expect([...shownBars("claude", {}, ["fiveHour", "weekly"])]).toEqual(["fiveHour", "weekly"]);
  });

  it("never none: when the bars left on have no data, the card shows what it has", () => {
    expect([...shownBars("codex", { codex: ["fiveHour"] }, ["fiveHour"])]).toEqual(["fiveHour"]);
    expect([
      ...shownBars("claude", { claude: ["fiveHour", "weekly"] }, ["fiveHour", "weekly"]),
    ]).toEqual(["fiveHour", "weekly"]);
  });
});
