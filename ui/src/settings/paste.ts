import { useEffect, useRef } from 'react';

type Handler = (text: string) => void;
const handlers: Array<{ current: Handler }> = [];

/**
 * Ctrl/Cmd+V on a Settings page (a theme, a branch preset): while the calling page is mounted,
 * the key pressed in Settings with the focus outside a text field or editor reads the clipboard
 * and hands its text to `handler`. The overlay owns the key, so the app's "Go to what's on the
 * clipboard" never sees it. The last-mounted handler wins.
 */
export function useSettingsPaste(handler: Handler): void {
  const ref = useRef(handler);
  ref.current = handler;
  useEffect(() => {
    handlers.push(ref);
    return () => {
      const i = handlers.lastIndexOf(ref);
      if (i >= 0) handlers.splice(i, 1);
    };
  }, []);
}

/** The handler Ctrl+V in Settings goes to, if a page registered one. */
export function settingsPasteHandler(): Handler | null {
  return handlers[handlers.length - 1]?.current ?? null;
}
