import { invoke, isTauri } from "@tauri-apps/api/core";

export interface NativePowerStatus {
  stayAwakeAvailable: boolean;
  stayAwake: boolean;
  screenOffAvailable: boolean;
}

/** Browser and unsupported desktops expose no power controls. */
export async function nativePowerStatus(): Promise<NativePowerStatus> {
  if (!isTauri()) return { stayAwakeAvailable: false, stayAwake: false, screenOffAvailable: false };
  return invoke<NativePowerStatus>("cockpit_power_status");
}

export async function setNativeStayAwake(enabled: boolean): Promise<void> {
  if (!isTauri()) throw new Error("Stay awake requires the native Cockpit app");
  await invoke("cockpit_power_set_stay_awake", { enabled });
}

export async function nativeScreenOff(): Promise<void> {
  if (!isTauri()) throw new Error("Screen power control requires the native Cockpit app");
  await invoke("cockpit_power_screen_off");
}
