import { Terminal } from 'lucide-react';
import { api, errorMessage } from '../../api/client';
import { confirmAction } from '../../ui/ConfirmDialog';
import { useToast } from '../../ui/toastStore';
import { appSetting, resetAppSettings } from '../bindings';
import { switchControl } from '../frame/controls';
import { registerSettings, registerSettingsPage, type SettingsCtx } from '../registry';

function OpenLogsControl() {
  const open = () => api.openLogsFolder().catch((e: unknown) => useToast.getState().show(errorMessage(e), { error: true }));
  return <button type="button" id="input-openLogs" onClick={() => void open()}>Open logs folder</button>;
}

function ResetAllControl({ ctx }: { ctx: SettingsCtx }) {
  const reset = async () => {
    const ok = await confirmAction({
      title: 'Reset GitVolt settings?',
      body: 'Every setting in the GitVolt group goes back to its default. Profile and repository settings, custom themes, branch presets and file colours stay.',
      confirmLabel: 'Reset',
      arm: 'Click again to reset every GitVolt setting',
      danger: true,
    });
    if (ok) resetAppSettings(ctx);
  };
  return <button type="button" id="input-resetAll" className="danger" onClick={() => void reset()}>Reset all settings…</button>;
}

const offs = [
  registerSettingsPage({ id: 'advanced', label: 'Advanced', scope: 'app', order: 80, icon: Terminal, summary: "Logs and resetting GitVolt's settings." }),
  registerSettings([
    {
      id: 'debugLogging', label: 'Debug logging', page: 'advanced', group: 'Logs',
      keywords: 'log file verbose diagnostics troubleshooting',
      description: 'Writes debug-level detail to the log files, from now on. Replaces a RUST_LOG set at launch.',
      Control: switchControl('debugLogging'), binding: appSetting('debugLogging', false),
    },
    {
      id: 'openLogs', label: 'Logs folder', page: 'advanced', group: 'Logs', labelFor: 'group',
      keywords: 'log file folder directory diagnostics troubleshooting',
      description: 'Where GitVolt writes its log files.',
      Control: OpenLogsControl,
    },
    {
      id: 'resetAll', label: 'Reset all settings', page: 'advanced', group: 'Reset', labelFor: 'group',
      keywords: 'reset defaults restore factory',
      description: 'Puts every GitVolt setting back to its default. Profile and repository settings and what you made stay.',
      Control: ResetAllControl,
    },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
