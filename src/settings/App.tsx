import { closeSettings } from "@/lib/bridge";
import SettingsPanel from "./_components/SettingsPanel";

export default function SettingsPage() {
  return <SettingsPanel onClose={closeSettings} />;
}
