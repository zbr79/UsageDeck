"use client";

import SettingsModal from "@/app/components/SettingsModal";
import { settingsWindow } from "@/lib/desktop-bridge";

export default function SettingsPage() {
  return (
    <div className="w-screen h-screen">
      <SettingsModal
        onClose={() => settingsWindow()?.close?.()}
        onChanged={() => {}}
      />
    </div>
  );
}