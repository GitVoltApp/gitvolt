import { Keyboard, X } from 'lucide-react';
import { useEffect, useRef } from 'react';
import { create } from 'zustand';
import { useModalKeys } from '../app/modalKeys';
import { comboOf } from '../app/shortcuts';
import { registerKeys } from '../ui/keyRouter';
import { displayChord, resolveChord } from '../ui/platformKeys';
import { Keys, ShortcutsList } from './ShortcutsList';

export const useShortcutsUi = create<{ open: boolean; setOpen: (o: boolean) => void }>((set) => ({ open: false, setOpen: (open) => set({ open }) }));
const close = () => useShortcutsUi.getState().setOpen(false);

/** Ctrl+/: the Keyboard Shortcuts panel, built from the app's real registrations (`catalog.ts`). */
export function ShortcutsPanel() {
  const open = useShortcutsUi((s) => s.open);
  const ref = useModalKeys<HTMLDivElement>(open, close);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!open) return;
    // The modal claims every key in the menu layer; Ctrl+/ (toggle) and Ctrl+F (filter) are its own.
    return registerKeys('menu', (e) => {
      const c = comboOf(e);
      if (c === resolveChord('Mod+/')) { e.preventDefault(); close(); return 'handled'; }
      if (c === resolveChord('Mod+F')) { e.preventDefault(); input.current?.focus(); input.current?.select(); return 'handled'; }
    });
  }, [open]);
  if (!open) return null;
  return (
    <div className="modal-backdrop" onPointerDown={close}>
      <div ref={ref} className="modal sc-modal" role="dialog" aria-modal="true" aria-label="Keyboard Shortcuts" onPointerDown={(e) => e.stopPropagation()}>
        <div className="sc-head">
          <h2><Keyboard size={16} aria-hidden />Keyboard Shortcuts <Keys chord="Mod+/" /></h2>
          <button type="button" className="icon-button" aria-label="Close shortcuts" onClick={close}><X size={14} /></button>
        </div>
        <ShortcutsList inputRef={input} autoFocus placeholder={`Filter shortcuts (${displayChord('Mod+F')})`} />
      </div>
    </div>
  );
}
