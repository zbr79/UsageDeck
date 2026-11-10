"use client";

import { useId, type InputHTMLAttributes } from "react";
import { SETTINGS_TYPE_SCALE } from "@/lib/theme";

export default function Field({
  label,
  labelClassName = "",
  wrapperClassName = "",
  className = "",
  mono = false,
  id,
  ...props
}: InputHTMLAttributes<HTMLInputElement> & {
  mono?: boolean;
  label?: string;
  labelClassName?: string;
  wrapperClassName?: string;
}) {
  const autoId = useId();
  const inputId = id ?? autoId;
  const inputEl = (
    <input
      id={inputId}
      {...props}
      className={`bg-role-surface border border-role-field ${SETTINGS_TYPE_SCALE.body} text-role-text placeholder:text-role-muted ${
        mono ? "font-mono" : ""
      } ${className}`}
    />
  );

  if (!label) return inputEl;

  return (
    <div className={wrapperClassName || "flex flex-col gap-1"}>
      <label
        htmlFor={inputId}
        className={labelClassName || `${SETTINGS_TYPE_SCALE.caption} text-role-muted shrink-0`}
      >
        {label}
      </label>
      {inputEl}
    </div>
  );
}
