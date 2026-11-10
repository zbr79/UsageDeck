"use client";

import type { ButtonHTMLAttributes } from "react";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";

export default function Button({
  variant = "primary",
  className = "",
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant }) {
  const variantClass =
    variant === "primary"
      ? "bg-role-text text-role-bg hover:opacity-90"
      : variant === "secondary"
        ? "bg-role-surface text-role-text border border-role-border hover:opacity-90"
        : variant === "danger"
          ? "bg-red-600 hover:bg-red-500 text-white"
          : "text-role-muted hover:text-role-text hover:bg-role-surface";

  return (
    <button
      {...props}
      className={`rounded-lg transition-colors disabled:opacity-50 ${variantClass} ${className}`}
    >
      {children}
    </button>
  );
}
