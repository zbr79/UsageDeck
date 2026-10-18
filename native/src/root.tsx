// Shared page shell (was app/layout.tsx): global CSS plus the toast host.
import { StrictMode, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import ToastProvider from "@/components/toast/ToastProvider";
import "./globals.css";

export function mount(page: ReactNode) {
  const el = document.getElementById("root");
  if (!el) throw new Error("missing #root");
  createRoot(el).render(
    <StrictMode>
      {page}
      <ToastProvider />
    </StrictMode>,
  );
}
