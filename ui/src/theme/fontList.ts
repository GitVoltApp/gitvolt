import { create } from 'zustand';
import { api } from '../api/client';
import type { FontFamily } from '../api/gen/FontFamily';

/**
 * The installed font families, for the font pickers: `null` until the first load, which the
 * first picker on screen starts (never at startup: listing reads every font file's header, once
 * per session in the core). A failed load leaves `null` and is retried on the next.
 */
export const useFontFamilies = create<{ list: FontFamily[] | null; loading: boolean }>(() => ({ list: null, loading: false }));

let inflight: Promise<void> | undefined;

function load(refresh: boolean): Promise<void> {
  if (inflight) return inflight;
  useFontFamilies.setState({ loading: true });
  inflight = Promise.resolve()
    .then(() => api.fontFamilies(refresh))
    .then((list) => { useFontFamilies.setState({ list }); }, () => {})
    .finally(() => {
      inflight = undefined;
      useFontFamilies.setState({ loading: false });
    });
  return inflight;
}

/** Loads the list if it never was. */
export const loadFontFamilies = (): Promise<void> => (useFontFamilies.getState().list ? Promise.resolve() : load(false));
/** Lists the fonts again (a font installed since appears). */
export const refreshFontFamilies = (): Promise<void> => load(true);
