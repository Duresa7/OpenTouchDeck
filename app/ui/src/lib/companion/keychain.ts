// Where the Companion keeps secrets such as API keys, worded for the platform it runs on. The
// Companion uses the system keychain and never falls back to a file; its own error text names
// no platform, so every platform word the UI shows comes from here.
import { useContext, useEffect, useState } from 'react';
import type { Platform } from './api.ts';
import type { Companion } from './client.ts';
import { CompanionContext } from './connection.ts';

export function keychainName(platform: Platform | null): string {
  switch (platform) {
    case 'windows':
      return 'Windows Credential Manager';
    case 'linux':
      return 'your system keychain (GNOME Keyring or KWallet)';
    default:
      return 'your system keychain';
  }
}

/** What to do when the keychain can't be reached. */
export function keychainMissing(platform: Platform | null): string {
  switch (platform) {
    case 'windows':
      return "Windows Credential Manager isn't available. Restart Open TouchDeck and try again.";
    case 'linux':
      return 'No system keychain is available. Start GNOME Keyring or KWallet and try again.';
    default:
      return 'No system keychain is available. Start it and try again.';
  }
}

const platforms = new WeakMap<object, Promise<Platform | null>>();

/** The Companion's platform, asked once per connection. Null when the Companion didn't say. */
export function platformOf(companion: Pick<Companion, 'system'>): Promise<Platform | null> {
  let platform = platforms.get(companion);
  if (!platform) {
    platform = companion.system.get().then(
      (info) => info.platform,
      () => {
        // Ask again next time.
        platforms.delete(companion);
        return null;
      },
    );
    platforms.set(companion, platform);
  }
  return platform;
}

/** The Companion's platform, or null until it answers or without a Companion. */
export function usePlatform(): Platform | null {
  const companion = useContext(CompanionContext);
  const [platform, setPlatform] = useState<Platform | null>(null);
  useEffect(() => {
    if (!companion) return;
    let live = true;
    void platformOf(companion).then((p) => live && setPlatform(p));
    return () => {
      live = false;
    };
  }, [companion]);
  return platform;
}

/** The keychain's name for this Companion's platform. */
export function useKeychainName(): string {
  return keychainName(usePlatform());
}
