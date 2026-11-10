"use client";

import type { ReactNode } from "react";

export type CardState = "default" | "inUse" | "exhausted" | "disabled";

export default function CardShell({
  state = "default",
  light,
  children,
}: {
  state?: CardState;
  light: boolean;
  cardBg?: string;
  children: ReactNode;
}) {
  const borderClass =
    state === "disabled"
      ? "border border-role-border opacity-55"
      : state === "exhausted"
        ? light
          ? "border border-red-600/85 shadow-[0_0_14px_rgba(239,68,68,0.45)]"
          : "border border-red-400/75 shadow-[0_0_14px_rgba(239,68,68,0.45)]"
        : state === "inUse"
          ? light
            ? "border border-emerald-600/85 shadow-[0_0_14px_rgba(16,185,129,0.45)]"
            : "border border-emerald-400/75 shadow-[0_0_14px_rgba(16,185,129,0.45)]"
          : "border border-role-border";

  return (
    <div className={`rounded-lg p-2 transition-colors duration-700 bg-role-bg ${borderClass}`}>
      {children}
    </div>
  );
}
