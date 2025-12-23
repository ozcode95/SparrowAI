/**
 * Autostart Implementation Example for SparrowAI
 *
 * This file demonstrates how to use the autostart functionality
 * in your Tauri application using both methods:
 * 1. JavaScript API from @tauri-apps/plugin-autostart
 * 2. Custom Rust backend commands
 */

import { enable, isEnabled, disable } from "@tauri-apps/plugin-autostart";
import { invoke } from "@tauri-apps/api/core";

// ============================================================================
// Method 1: Using JavaScript Plugin API (Direct Access)
// ============================================================================

/**
 * Enable autostart using the JavaScript plugin API
 */
export async function enableAutostartJS() {
  try {
    await enable();
    console.log("Autostart enabled successfully (JS API)");
  } catch (error) {
    console.error("Failed to enable autostart:", error);
  }
}

/**
 * Disable autostart using the JavaScript plugin API
 */
export async function disableAutostartJS() {
  try {
    await disable();
    console.log("Autostart disabled successfully (JS API)");
  } catch (error) {
    console.error("Failed to disable autostart:", error);
  }
}

/**
 * Check if autostart is enabled using the JavaScript plugin API
 */
export async function checkAutostartJS(): Promise<boolean> {
  try {
    const enabled = await isEnabled();
    console.log(`Autostart is ${enabled ? "enabled" : "disabled"} (JS API)`);
    return enabled;
  } catch (error) {
    console.error("Failed to check autostart status:", error);
    return false;
  }
}

// ============================================================================
// Method 2: Using Custom Rust Backend Commands
// ============================================================================

/**
 * Enable autostart using the Rust backend command
 */
export async function enableAutostartRust() {
  try {
    await invoke("enable_autostart");
    console.log("Autostart enabled successfully (Rust backend)");
  } catch (error) {
    console.error("Failed to enable autostart:", error);
  }
}

/**
 * Disable autostart using the Rust backend command
 */
export async function disableAutostartRust() {
  try {
    await invoke("disable_autostart");
    console.log("Autostart disabled successfully (Rust backend)");
  } catch (error) {
    console.error("Failed to disable autostart:", error);
  }
}

/**
 * Check if autostart is enabled using the Rust backend command
 */
export async function checkAutostartRust(): Promise<boolean> {
  try {
    const enabled = await invoke<boolean>("is_autostart_enabled");
    console.log(
      `Autostart is ${enabled ? "enabled" : "disabled"} (Rust backend)`
    );
    return enabled;
  } catch (error) {
    console.error("Failed to check autostart status:", error);
    return false;
  }
}

/**
 * Toggle autostart on/off using the Rust backend command
 */
export async function toggleAutostartRust(): Promise<boolean> {
  try {
    const newState = await invoke<boolean>("toggle_autostart");
    console.log(`Autostart toggled: now ${newState ? "enabled" : "disabled"}`);
    return newState;
  } catch (error) {
    console.error("Failed to toggle autostart:", error);
    return false;
  }
}

// ============================================================================
// UI Integration Example
// ============================================================================

/**
 * Example React component usage
 */
export function AutostartSettingsExample() {
  const [autostartEnabled, setAutostartEnabled] = React.useState(false);

  React.useEffect(() => {
    // Check current autostart status on mount
    checkAutostartRust().then(setAutostartEnabled);
  }, []);

  const handleToggle = async () => {
    const newState = await toggleAutostartRust();
    setAutostartEnabled(newState);
  };

  return (
    <div>
      <label>
        <input
          type="checkbox"
          checked={autostartEnabled}
          onChange={handleToggle}
        />
        Launch SparrowAI on system startup
      </label>
    </div>
  );
}

/**
 * Example: Initialize autostart settings
 */
export async function initializeAutostartSettings() {
  const isEnabled = await checkAutostartRust();

  // You can store this in your app state/settings
  return {
    autostartEnabled: isEnabled,
  };
}

/**
 * Example: Integrate with settings page
 */
export async function updateAutostartSetting(enable: boolean) {
  if (enable) {
    await enableAutostartRust();
  } else {
    await disableAutostartRust();
  }
}
