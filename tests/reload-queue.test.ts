import { describe, expect, it, vi } from "vitest";
import {
  burstPoller,
  FAST_POLL_FOR_MS,
  FAST_POLL_MS,
  queuedRunner,
} from "@/widget/_lib/useWidgetData";

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

describe("burstPoller (after Open Cursor)", () => {
  it("loads every 10 s for 2 minutes, then stops", () => {
    vi.useFakeTimers();
    try {
      let loads = 0;
      const poller = burstPoller(
        () => loads++,
        () => false,
      );
      poller.start();
      vi.advanceTimersByTime(FAST_POLL_MS * 3);
      expect(loads).toBe(3);
      vi.advanceTimersByTime(FAST_POLL_FOR_MS);
      expect(loads).toBe(FAST_POLL_FOR_MS / FAST_POLL_MS - 1);
      vi.advanceTimersByTime(FAST_POLL_FOR_MS);
      expect(loads, "no loads after the window").toBe(FAST_POLL_FOR_MS / FAST_POLL_MS - 1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("stops as soon as the sign-in is back, and restarts on the next start", () => {
    vi.useFakeTimers();
    try {
      let loads = 0;
      let signedIn = false;
      const poller = burstPoller(
        () => loads++,
        () => signedIn,
      );
      poller.start();
      vi.advanceTimersByTime(FAST_POLL_MS * 2);
      signedIn = true;
      vi.advanceTimersByTime(FAST_POLL_MS * 5);
      expect(loads).toBe(2);
      signedIn = false;
      poller.start();
      vi.advanceTimersByTime(FAST_POLL_MS);
      expect(loads).toBe(3);
      poller.stop();
      vi.advanceTimersByTime(FAST_POLL_MS * 5);
      expect(loads).toBe(3);
    } finally {
      vi.useRealTimers();
    }
  });
});
