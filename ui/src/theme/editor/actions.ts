import { useMemo } from 'react';
import type { CustomTheme } from '../../api/gen/CustomTheme';
import { api, errorMessage } from '../../api/client';
import { copyText } from '../../api/transport';
import { useAppState } from '../../app/state';
import { exportFileName } from '../../settings/share';
import { useToast, type ToastOptions } from '../../ui/toastStore';
import { themeDef } from '../custom';
import { themeFromJson, themeToJson } from '../themeJson';
import type { ThemeDef } from '../themes';

const NO_CUSTOMS: CustomTheme[] = [];

/** The theme Settings › Appearance edits (the saved one, resolved), and its custom theme when it is one. */
export function useEditedTheme(): { def: ThemeDef; custom: CustomTheme | undefined; customs: readonly CustomTheme[] } {
  const theme = useAppState((s) => s.settings.theme);
  const customs = useAppState((s) => s.settings.customThemes) ?? NO_CUSTOMS;
  return useMemo(() => {
    const def = themeDef(theme, customs);
    return { def, custom: customs.find((c) => c.id === def.id), customs };
  }, [theme, customs]);
}

const toast = (msg: string, opts?: ToastOptions) => useToast.getState().show(msg, opts);

export async function copyTheme(def: ThemeDef, custom: CustomTheme): Promise<void> {
  try {
    await copyText(themeToJson(def, custom));
    toast(`Copied ${def.label} as JSON`);
  } catch (e) {
    toast(errorMessage(e), { error: true });
  }
}

/** Saves the theme as a file where the user picks; cancelling the dialog says nothing. */
export async function exportTheme(def: ThemeDef, custom: CustomTheme): Promise<void> {
  try {
    const path = await api.saveExport(exportFileName(def.label, 'theme'), themeToJson(def, custom));
    if (path) toast(`Saved ${path.split(/[\\/]/).pop()}`, { detail: path });
  } catch (e) {
    toast(errorMessage(e), { error: true });
  }
}

/** A pasted theme (untrusted text): added and selected, or a toast and no change. */
export function pasteTheme(text: string): void {
  const customs = useAppState.getState().settings.customThemes ?? [];
  const theme = themeFromJson(text, customs);
  if (!theme) {
    toast("The clipboard doesn't hold a GitVolt theme");
    return;
  }
  useAppState.getState().setSettings({ customThemes: [...customs, theme], theme: theme.id });
  toast(`Added ${theme.name}`);
}
