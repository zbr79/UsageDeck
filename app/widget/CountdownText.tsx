"use client";

import { useEffect, useState, useSyncExternalStore } from "react";
import { fmtDuration, getPeakInfo, resetFmt } from "@/lib/ranking";
import { WIDGET_TYPE_SCALE } from "@/lib/theme";

let currentNow = Date.now();
const listeners = new Set<() => void>();
let intervalId: ReturnType<typeof setInterval> | null = null;

function subscribeNow(listener: () => void) {
  listeners.add(listener);
  if (intervalId === null && typeof window !== "undefined") {
    intervalId = setInterval(() => {
      currentNow = Date.now();
      for (const l of listeners) l();
    }, 1000);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && intervalId !== null) {
      clearInterval(intervalId);
      intervalId = null;
    }
  };
}

function getNowSnapshot() {
  return currentNow;
}

export function useNowSecond(): number {
  return useSyncExternalStore(subscribeNow, getNowSnapshot, getNowSnapshot);
}

export function useRecentIncrease(at: number | undefined, windowMs = 20_000): boolean {
  const [expiredAt, setExpiredAt] = useState<number | null>(null);

  useEffect(() => {
    if (!at) return;
    const remaining = Math.max(0, windowMs - (Date.now() - at));
    const timer = setTimeout(() => setExpiredAt(at), remaining);
    return () => clearTimeout(timer);
  }, [at, windowMs]);

  return Boolean(at && expiredAt !== at);
}

export function ResetTicker({
  iso,
  prefix = "",
  className = "",
}: {
  iso: string | null | undefined;
  prefix?: string;
  className?: string;
}) {
  const now = useNowSecond();
  if (!iso) return null;
  const text = resetFmt(iso, now);
  if (!text) return null;
  return (
    <div className={className}>
      {prefix}
      {text}
    </div>
  );
}

export function PeakHourBadge({ enabled }: { enabled: boolean }) {
  const now = useNowSecond();
  if (!enabled) return null;
  const peakInfo = getPeakInfo(new Date(now));
  if (peakInfo.active && peakInfo.endAt) {
    return (
      <span className={`${WIDGET_TYPE_SCALE.caption} font-medium whitespace-nowrap text-role-danger`}>
        Peak Ends in {fmtDuration(peakInfo.endAt - now)}
      </span>
    );
  }
  if (peakInfo.nextStartAt) {
    return (
      <span className={`${WIDGET_TYPE_SCALE.caption} font-medium whitespace-nowrap text-role-success`}>
        Peak Starts in {fmtDuration(peakInfo.nextStartAt - now)}
      </span>
    );
  }
  return null;
}
