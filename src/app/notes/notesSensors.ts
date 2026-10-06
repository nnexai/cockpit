import { KeyboardSensor, MouseSensor, TouchSensor, type Sensor, type SensorInstance, type SensorOptions, type SensorProps } from "@dnd-kit/core";

interface CancellationOptions {
  registerCancellation(cancel: () => void): () => void;
}

/**
 * dnd-kit 6.3.1 exposes no cancellation lifecycle on SensorInstance. Its real
 * handleCancel removes keyboard/pointer listeners before calling onCancel.
 * Keep this one compatibility boundary with the exact pinned dependency; never
 * merely hide/remount a drag, which leaves listeners able to swallow typing or
 * complete an already-cancelled move after a Space change.
 */
function cancellableSensor<Options extends SensorOptions>(Driver: Sensor<Options>): Sensor<Options & CancellationOptions> {
  const Adapter: Sensor<Options & CancellationOptions> = class implements SensorInstance {
    static activators = Driver.activators;
    private driver: SensorInstance;
    get autoScrollEnabled() { return this.driver.autoScrollEnabled; }
    constructor(props: SensorProps<Options & CancellationOptions>) {
      let release: (() => void) | undefined;
      this.driver = new Driver({ ...props,
        onAbort: id => { release?.(); props.onAbort(id); },
        onEnd: () => { release?.(); props.onEnd(); },
        onCancel: () => { release?.(); props.onCancel(); },
      });
      const cancel = Reflect.get(this.driver, "handleCancel");
      if (typeof cancel !== "function") throw new Error("Pinned dnd-kit sensor cancellation contract changed");
      release = props.options.registerCancellation(() => {
        // This event is passed directly to the sensor, never dispatched as input.
        cancel.call(this.driver, new Event("notes-sensor-cancel", { cancelable: true }));
      });
    }
  };
  if (Driver.setup) Adapter.setup = Driver.setup;
  return Adapter;
}

export const NotesMouseSensor = cancellableSensor(MouseSensor);
export const NotesTouchSensor = cancellableSensor(TouchSensor);
export const NotesKeyboardSensor = cancellableSensor(KeyboardSensor);
