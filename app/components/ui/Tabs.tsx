"use client";

import { useRef, type KeyboardEvent, type ReactNode } from "react";
import { SETTINGS_TYPE_SCALE } from "@/lib/theme";

export interface TabItem<T extends string> {
  id: T;
  label: string;
  icon?: ReactNode;
}

export function tabDomId(idPrefix: string, id: string): string {
  return `${idPrefix}-${id}`;
}

export function tabPanelDomId(idPrefix: string, id: string): string {
  return `${idPrefix}-panel-${id}`;
}

export default function Tabs<T extends string>({
  items,
  active,
  onSelect,
  ariaLabel,
  boldActive = false,
  idPrefix = "tab",
}: {
  items: TabItem<T>[];
  active: T;
  onSelect: (id: T) => void;
  ariaLabel?: string;
  boldActive?: boolean;
  idPrefix?: string;
}) {
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);

  const handleKeyDown = (e: KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (items.length === 0) return;
    let nextIndex: number | null = null;
    if (e.key === "ArrowRight") {
      nextIndex = (index + 1) % items.length;
    } else if (e.key === "ArrowLeft") {
      nextIndex = (index - 1 + items.length) % items.length;
    } else if (e.key === "Home") {
      nextIndex = 0;
    } else if (e.key === "End") {
      nextIndex = items.length - 1;
    }
    if (nextIndex !== null) {
      e.preventDefault();
      const nextItem = items[nextIndex];
      onSelect(nextItem.id);
      tabRefs.current[nextIndex]?.focus();
    }
  };

  return (
    <div
      role="tablist"
      aria-label={ariaLabel}
      className="flex gap-5 px-5 border-b border-role-border mb-3"
    >
      {items.map((item, idx) => {
        const isSelected = active === item.id;
        return (
          <button
            key={item.id}
            ref={(el) => {
              tabRefs.current[idx] = el;
            }}
            type="button"
            role="tab"
            id={tabDomId(idPrefix, item.id)}
            aria-controls={tabPanelDomId(idPrefix, item.id)}
            aria-selected={isSelected}
            tabIndex={isSelected ? 0 : -1}
            onClick={() => onSelect(item.id)}
            onKeyDown={(e) => handleKeyDown(e, idx)}
            className={`${item.icon ? "flex items-center gap-1.5 " : ""}px-0.5 pb-2 pt-1 -mb-px ${SETTINGS_TYPE_SCALE.body} border-b-2 transition-colors ${
              isSelected
                ? `border-role-accent text-role-text${boldActive ? " font-medium" : ""}`
                : "border-transparent text-role-muted hover:text-role-text"
            }`}
          >
            {item.icon}
            {item.label}
          </button>
        );
      })}
    </div>
  );
}
