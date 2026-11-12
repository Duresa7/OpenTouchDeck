import { useLayoutEffect } from 'react';
import './window-frame.css';

interface Overlay extends EventTarget { visible: boolean }

/** Browser tabs and framed windows reserve no title bar space. */
export function WindowFrame() {
  useLayoutEffect(() => {
    const overlay = (navigator as Navigator & { windowControlsOverlay?: Overlay }).windowControlsOverlay;
    if (!window.openTouchDeck || !overlay) return;
    const update = () => document.documentElement.toggleAttribute('data-window-overlay', overlay.visible);
    update();
    overlay.addEventListener('geometrychange', update);
    return () => {
      overlay.removeEventListener('geometrychange', update);
      document.documentElement.removeAttribute('data-window-overlay');
    };
  }, []);
  return <div className="window-drag-strip" aria-hidden="true" />;
}
