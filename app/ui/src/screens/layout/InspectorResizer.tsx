import { useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type RefObject } from 'react';
import { clampWidth, INSPECTOR_DEFAULT, INSPECTOR_MIN, inspectorLimits, keyWidth, readWidth, writeWidth, type InspectorLimits } from './inspector-width.ts';

/**
 * The details panel's width, clamped to what the editor can give it and kept clamped as the window resizes.
 * `reserved` says what an editor this wide gives something else first: the Pages column while it's out.
 */
export function useInspectorWidth(editor: RefObject<HTMLElement | null>, reserved: (editorWidth: number) => number = () => 0) {
  const [editorWidth, setEditorWidth] = useState(() => (typeof window === 'undefined' ? 1280 : window.innerWidth));
  const [chosen, setChosen] = useState(() => readWidth());
  useLayoutEffect(() => {
    const el = editor.current;
    if (!el) return;
    const measure = () => setEditorWidth(el.clientWidth);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [editor]);
  const limits = inspectorLimits(editorWidth - reserved(editorWidth));
  const width = limits.fixed ? INSPECTOR_MIN : clampWidth(chosen, limits);
  return {
    editorWidth,
    limits,
    width,
    choose(next: number) {
      setChosen(next);
      writeWidth(next);
    },
  };
}

interface Drag {
  pointer: number;
  startX: number;
  startWidth: number;
  next: number;
  frame: number;
}

/**
 * The handle on the details panel's left edge. Dragging follows the pointer 1:1 and writes the width straight to the
 * editor's CSS, at most once a frame, with no render per move; the width is kept when the drag ends. Arrow keys,
 * Home, End and a double-click reset work too.
 */
export function InspectorResizer({ editor, width, limits, onWidth }: { editor: RefObject<HTMLElement | null>; width: number; limits: InspectorLimits; onWidth(width: number): void }) {
  const drag = useRef<Drag | null>(null);
  const handle = useRef<HTMLDivElement>(null);

  const end = (event: PointerEvent<HTMLDivElement>) => {
    const done = drag.current;
    if (!done || event.pointerId !== done.pointer) return;
    cancelAnimationFrame(done.frame);
    drag.current = null;
    handle.current?.removeAttribute('data-dragging');
    document.body.classList.remove('is-resizing');
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    onWidth(done.next);
  };

  return (
    <div
      ref={handle}
      className="inspector-resizer"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize details panel"
      aria-valuenow={width}
      aria-valuemin={limits.min}
      aria-valuemax={limits.max}
      tabIndex={0}
      data-testid="inspector-resizer"
      onPointerDown={(event) => {
        if (event.button !== 0 || drag.current) return;
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = { pointer: event.pointerId, startX: event.clientX, startWidth: width, next: width, frame: 0 };
        handle.current?.setAttribute('data-dragging', '');
        document.body.classList.add('is-resizing');
      }}
      onPointerMove={(event) => {
        const now = drag.current;
        if (!now || event.pointerId !== now.pointer) return;
        now.next = clampWidth(now.startWidth + now.startX - event.clientX, limits);
        if (now.frame) return;
        now.frame = requestAnimationFrame(() => {
          now.frame = 0;
          editor.current?.style.setProperty('--inspector-w', `${now.next}px`);
          handle.current?.setAttribute('aria-valuenow', String(now.next));
        });
      }}
      onPointerUp={end}
      onPointerCancel={end}
      onLostPointerCapture={end}
      onDoubleClick={() => onWidth(clampWidth(INSPECTOR_DEFAULT, limits))}
      onKeyDown={(event: KeyboardEvent<HTMLDivElement>) => {
        const next = keyWidth(width, event.key, event.shiftKey, limits);
        if (next === null) return;
        event.preventDefault();
        event.stopPropagation();
        onWidth(next);
      }}
    />
  );
}
