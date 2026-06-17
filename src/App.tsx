import { useEffect } from "react";
import { Shell } from "./components/layout/Shell";
import { useUiStore } from "./stores/ui";
import { useSystemStore } from "./stores/system";
import { useSettingsStore } from "./stores/settings";

export default function App() {
  const bindHotkeys = useUiStore((s) => s.bindHotkeys);
  const probeSystem = useSystemStore((s) => s.probe);
  const loadSettings = useSettingsStore((s) => s.load);

  useEffect(() => {
    const unbind = bindHotkeys();
    void loadSettings();
    void probeSystem();
    return unbind;
  }, [bindHotkeys, probeSystem, loadSettings]);

  return <Shell />;
}
