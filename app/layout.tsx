import type { Metadata } from "next";
import "./globals.css";
import ToastProvider from "@/app/components/toast/ToastProvider";

export const metadata: Metadata = {
  title: "Provider Usage Monitor",
  description: "Usage monitor for OpenCode Go, Cursor, Grok Bot, Codex and Claude",
};

export default function RootLayout({ children }: LayoutProps<"/">) {
  return (
    <html lang="en" className="h-full antialiased">
      <body className="min-h-full flex flex-col">
        {children}
        <ToastProvider />
      </body>
    </html>
  );
}
