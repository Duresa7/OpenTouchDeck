import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import clsx from 'clsx';
import { Check, LoaderCircle, Upload } from 'lucide-react';
import { saveState, type EditorStore } from '../../lib/editor/store.ts';
import { button } from '../../shell/button.ts';
import { useEditorControlNotice } from './Controls.tsx';
import { useEditorState } from './editor.ts';
import { Tip } from './Tip.tsx';

/** How long "Saved" shows before the button folds away. */
export const SAVED_MS = 1600;

type Look = 'send' | 'sending' | 'saved';

const LOOKS: Record<Look, { icon: typeof Upload; words: string }> = {
  send: { icon: Upload, words: 'Send to Device' },
  sending: { icon: LoaderCircle, words: 'Sending…' },
  saved: { icon: Check, words: 'Saved' },
};

/**
 * Send to Device shows while this Layout has changes the Device lacks, or while its controls need a send. A click
 * turns it into a spinner, then "Saved" for a moment, then it folds away until the next change. Its width follows
 * the words, so each step slides rather than jumps. Sending again without a change is in the Layout menu.
 */
export function SendButton({ store, name, live }: { store: EditorStore; name: string; live: boolean }) {
  const saved = useEditorState(store, saveState);
  const busy = useEditorState(store, (s) => s.busy);
  const needsSend = useEditorControlNotice(store).kind === 'needs-send';
  const [flash, setFlash] = useState(false);
  const before = useRef(saved);

  useEffect(() => {
    const was = before.current;
    before.current = saved;
    // A failed send of a Layout the Device already had also ends on 'saved', so only a send that worked says Saved.
    if (was === 'sending' && saved === 'saved' && store.getState().status?.event === 'pushed') {
      setFlash(true);
      const done = setTimeout(() => setFlash(false), SAVED_MS);
      return () => clearTimeout(done);
    }
    if (saved !== 'saved') setFlash(false);
    return undefined;
  }, [saved, store]);

  const look: Look = saved === 'sending' ? 'sending' : flash ? 'saved' : 'send';
  const shown = saved === 'unsent' || saved === 'sending' || flash || needsSend;
  const canSend = look === 'send' && live && busy === null;

  // The button's width follows its words, measured, so the change can slide.
  const inner = useRef<HTMLSpanElement>(null);
  const [width, setWidth] = useState<number | null>(null);
  useLayoutEffect(() => {
    const el = inner.current;
    if (!el) return;
    const measure = () => setWidth(el.getBoundingClientRect().width);
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => watch.disconnect();
  }, [look]);

  const { icon: Icon, words } = LOOKS[look];
  const style = width === null ? undefined : ({ '--push-w': `${width}px` } as CSSProperties);
  const tip = look === 'send' ? (live ? `Send this Layout to ${name}` : `${name} isn't connected`) : words;
  return (
    <Tip label={tip}>
      <button
        type="button"
        className={clsx(button({ kind: 'primary' }), 'push-button')}
        style={style}
        aria-disabled={!canSend}
        aria-hidden={shown ? undefined : true}
        inert={!shown}
        tabIndex={shown ? undefined : -1}
        onClick={() => canSend && void store.push()}
        data-state={shown ? look : 'hidden'}
        data-testid="push"
      >
        <span ref={inner} className="push-inner" key={look}>
          <Icon className={clsx(look === 'sending' && 'push-spinner')} size={16} strokeWidth={2} aria-hidden />
          <span className="push-words">{words}</span>
        </span>
      </button>
    </Tip>
  );
}
