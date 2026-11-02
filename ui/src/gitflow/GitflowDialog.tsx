import { useState, type ReactNode } from 'react';
import { create } from 'zustand';
import type { GitflowConfig } from '../api/gen/GitflowConfig';
import type { GitflowKind } from '../api/gen/GitflowKind';
import { useModalKeys } from '../app/modalKeys';
import { useRuntime } from '../app/runtime';
import { registerAppSlot } from '../app/slots';
import { ctxOf, finishGitflow, flowOf, initGitflow, startGitflow } from './actions';
import { tagNameError } from '../tags/tagName';
import { baseOf, classify, configError, KIND_LABEL, KINDS, prefixOf, startNameError, suggestedTag, targetsOf } from './model';
import './gitflow.css';

/** One of git flow's three dialogs, for a tab: its settings (Init, or editing them), a new
 * branch, or a finish's options. */
type Req =
  | { kind: 'init'; tabId: string; initial: GitflowConfig; editing: boolean }
  | { kind: 'start'; tabId: string; flow: GitflowKind }
  | { kind: 'finish'; tabId: string; branch: string };

const useDialog = create<{ req: Req | null; seq: number }>(() => ({ req: null, seq: 0 }));
export const openGitflowDialog = (req: Req) => useDialog.setState((s) => ({ req, seq: s.seq + 1 }));
const close = () => useDialog.setState({ req: null });

export function GitflowDialog() {
  const { req, seq } = useDialog();
  if (!req) return null;
  if (req.kind === 'init') return <InitForm key={seq} req={req} />;
  if (req.kind === 'start') return <StartForm key={seq} req={req} />;
  return <FinishForm key={seq} req={req} />;
}

function Modal({ title, children }: { title: string; children: ReactNode }) {
  const ref = useModalKeys<HTMLDivElement>(true, close);
  return (
    <div className="modal-backdrop" onPointerDown={close}>
      <div ref={ref} className="modal gitflow-modal" role="dialog" aria-modal="true" aria-labelledby="gitflow-title" onPointerDown={(e) => e.stopPropagation()}>
        <h2 id="gitflow-title">{title}</h2>
        {children}
      </div>
    </div>
  );
}

const FIELDS: Array<[keyof GitflowConfig, string]> = [
  ['master', 'Production branch'],
  ['develop', 'Development branch'],
  ['feature', 'Feature prefix'],
  ['release', 'Release prefix'],
  ['hotfix', 'Hotfix prefix'],
  ['versionTag', 'Version tag prefix'],
];

function InitForm({ req }: { req: Extract<Req, { kind: 'init' }> }) {
  const [c, setC] = useState(req.initial);
  const locals = useRuntime((s) => s.tabs[req.tabId]?.sidebar?.locals);
  const error = configError(c);
  const missing = !!locals && !error && !locals.some((b) => b.name === c.master);
  const creates = !!locals && !error && !missing && !locals.some((b) => b.name === c.develop);
  const changed = FIELDS.some(([k]) => c[k] !== req.initial[k]);
  const submit = () => {
    const ctx = ctxOf(req.tabId);
    if (!ctx || error || missing || (req.editing && !changed)) return;
    close();
    void initGitflow(ctx, c, req.editing);
  };
  return (
    <Modal title={req.editing ? 'Git flow settings' : 'Set up git flow'}>
      <form onSubmit={(e) => { e.preventDefault(); submit(); }}>
        <div className="gitflow-grid">
          {FIELDS.map(([k, label], i) => (
            <label key={k} className="modal-field">
              <span>{label}</span>
              <input aria-label={label} value={c[k]} autoFocus={i === 0} spellCheck={false} placeholder={k === 'versionTag' ? 'None' : undefined} onChange={(e) => setC({ ...c, [k]: e.target.value })} />
            </label>
          ))}
        </div>
        {error && <p role="alert" className="modal-error">{error}</p>}
        {missing && <p role="alert" className="modal-error">There's no branch {c.master}: create it first</p>}
        {creates && <p>{c.develop} doesn't exist yet: it's created at {c.master}.</p>}
        <p>Kept in this repository's git config, where the git flow command line reads them too.</p>
        <div className="modal-actions">
          <button type="button" onClick={close}>Cancel</button>
          <button type="submit" disabled={!!error || missing || (req.editing && !changed)}>{req.editing ? 'Save' : 'Set up git flow'}</button>
        </div>
      </form>
    </Modal>
  );
}

function StartForm({ req }: { req: Extract<Req, { kind: 'start' }> }) {
  const config = flowOf(req.tabId);
  const locals = useRuntime((s) => s.tabs[req.tabId]?.sidebar?.locals);
  const [kind, setKind] = useState<GitflowKind>(req.flow);
  const [name, setName] = useState('');
  if (!config) return null;
  const error = startNameError(config, kind, name, (b) => !!locals?.some((l) => l.name === b));
  const base = baseOf(config, kind);
  const submit = () => {
    const ctx = ctxOf(req.tabId);
    if (!ctx || error) return;
    close();
    void startGitflow(ctx, config, kind, name);
  };
  return (
    <Modal title={`Start a ${KIND_LABEL[kind].toLowerCase()}`}>
      <form onSubmit={(e) => { e.preventDefault(); submit(); }}>
        <div className="gitflow-kinds" role="radiogroup" aria-label="Kind of branch">
          {KINDS.map((k) => (
            <label key={k} className="modal-check">
              <input type="radio" name="gitflow-kind" checked={kind === k} onChange={() => setKind(k)} /> {KIND_LABEL[k]}
            </label>
          ))}
        </div>
        <label className="modal-field">
          <span>{kind === 'feature' ? 'Name' : 'Version'}</span>
          <span className="gitflow-prefixed">
            <span className="gitflow-prefix" aria-hidden>{prefixOf(config, kind)}</span>
            <input aria-label={kind === 'feature' ? 'Feature name' : `${KIND_LABEL[kind]} version`} value={name} autoFocus spellCheck={false} onChange={(e) => setName(e.target.value)} />
          </span>
        </label>
        {error && name && <p role="alert" className="modal-error">{error}</p>}
        <p>Starts from {base} and checks the new branch out.</p>
        <div className="modal-actions">
          <button type="button" onClick={close}>Cancel</button>
          <button type="submit" disabled={!!error}>Start {KIND_LABEL[kind].toLowerCase()}</button>
        </div>
      </form>
    </Modal>
  );
}

function FinishForm({ req }: { req: Extract<Req, { kind: 'finish' }> }) {
  const config = flowOf(req.tabId);
  const flow = config && classify(config, req.branch);
  // A release or hotfix named after a version is tagged with it (`release/3.0.0`: `3.0.0`);
  // another name leaves the tag for the user to name. The message follows the tag until edited.
  const suggested = config && flow ? suggestedTag(config, flow.kind, flow.rest) : null;
  const [tag, setTag] = useState(suggested ?? '');
  const [message, setMessage] = useState<string | null>(null);
  const [keep, setKeep] = useState(false);
  if (!config || !flow) return null;
  const tagged = suggested !== null;
  const shownMessage = message ?? (tag ? `${KIND_LABEL[flow.kind]} ${tag}` : '');
  const tagError = tagged ? (tag ? tagNameError(tag) : 'Enter a tag name') : null;
  const targets = targetsOf(config, flow.kind);
  const steps = [`merges it into ${targets.join(', then ')}`, ...(tagged ? [`tags ${config.master}${tag && !tagError ? ` ${tag}` : ''}`] : []), ...(keep ? [] : ['deletes it'])];
  const submit = () => {
    const ctx = ctxOf(req.tabId);
    if (!ctx || tagError) return;
    close();
    void finishGitflow(ctx, req.branch, { tag: tagged ? tag : null, tagMessage: tagged && shownMessage.trim() ? shownMessage : null, keep });
  };
  return (
    <Modal title={`Finish ${req.branch}`}>
      <form onSubmit={(e) => { e.preventDefault(); submit(); }}>
        <p className="gitflow-steps">Finishing {steps.join(', ')}, then leaves you on {config.develop}.</p>
        {tagged && (
          <>
            <label className="modal-field">
              <span>Tag</span>
              <input aria-label="Tag" value={tag} autoFocus={!tag} spellCheck={false} placeholder="3.0.0" onChange={(e) => setTag(e.target.value.trim())} />
            </label>
            {tagError && tag && <p role="alert" className="modal-error">{tagError}</p>}
            <label className="modal-field">
              <span>Tag message</span>
              <input aria-label="Tag message" value={shownMessage} autoFocus={!!tag} spellCheck={false} placeholder="None: a lightweight tag" onChange={(e) => setMessage(e.target.value)} />
            </label>
          </>
        )}
        <label className="modal-check">
          <input type="checkbox" checked={!keep} onChange={(e) => setKeep(!e.target.checked)} /> Delete {req.branch}
        </label>
        <div className="modal-actions">
          <button type="button" onClick={close}>Cancel</button>
          <button type="submit" autoFocus={!tagged} disabled={!!tagError}>Finish {KIND_LABEL[flow.kind].toLowerCase()}</button>
        </div>
      </form>
    </Modal>
  );
}

export const offGitflowDialog = registerAppSlot('overlay', 'gitflow', GitflowDialog);
