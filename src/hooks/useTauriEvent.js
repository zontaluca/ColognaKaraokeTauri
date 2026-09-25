import { useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";

/**
 * Subscribe to a Tauri event for the lifetime of the component.
 *
 * - The handler is read through a ref, so passing an inline function does not
 *   re-subscribe on every render.
 * - `listen()` resolves asynchronously: if the component unmounts (or `enabled`
 *   flips) before it resolves, the listener is removed as soon as it arrives
 *   instead of leaking and firing twice (e.g. under React StrictMode).
 */
export function useTauriEvent(event, handler, enabled = true) {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;

  useEffect(() => {
    if (!enabled) return undefined;
    let disposed = false;
    let unlisten = null;
    listen(event, (ev) => handlerRef.current(ev)).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, [event, enabled]);
}
