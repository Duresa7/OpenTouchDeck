import type { BrowserWindowConstructorOptions } from 'electron';

/** Match the desk mat and ink tokens, including before the page has loaded. */
export function overlayColors(dark: boolean) {
  return dark ? { color: '#000000', symbolColor: '#eef1f5' } : { color: '#e4e6e9', symbolColor: '#1a1d22' };
}

export function frameOptions(platform: string, dark: boolean): BrowserWindowConstructorOptions {
  return platform === 'win32' || platform === 'linux'
    ? { titleBarStyle: 'hidden', titleBarOverlay: { ...overlayColors(dark), height: 32 } }
    : {};
}
