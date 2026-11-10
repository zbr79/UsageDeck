import { describe, expect, it } from "vitest";
import { queuedRunner } from "@/widget/_lib/useWidgetData";

describe("queuedRunner (widget reload)", () => {
  it("queues one follow-up load when a reload arrives during a poll", async () => {
    let runs = 0;
    let release: () => void = () => {};
    const load = queuedRunner(async () => {
      runs++;
      if (runs === 1) await new Promise<void>((r) => (release = r));
    });
    const first = load();
    const second = load(); // asked for while the first poll is in flight
    const third = load(); // folds into the same follow-up
    release();
    await Promise.all([first, second, third]);
    expect(runs).toBe(2);
    await load();
    expect(runs).toBe(3);
  });
});
