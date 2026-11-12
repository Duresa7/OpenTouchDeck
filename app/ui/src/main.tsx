import '@fontsource-variable/archivo/wdth.css';
import './styles/tokens.css';
import './styles/app.css';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { Pair, takePairingCode } from './screens/Pair.tsx';
import { WindowFrame } from './shell/WindowFrame.tsx';
import { App } from './App.tsx';
import { CompanionContext, connect, EventsContext, FolderPickerContext, pageEnvironment, type Environment } from './lib/companion/connection.ts';
import { reopenSettings } from './state/ui.ts';
import { createUpdatesStore, UpdatesContext, watchForUpdates } from './state/updates.ts';

const isPair = window.location.pathname === '/pair';
const initialCode = isPair ? takePairingCode(window.location, window.history) : '';

// The demo and dev branches are dropped from production builds, fake Companion and all.
const env: Environment =
  import.meta.env.MODE === 'demo'
    ? (await import('./lib/companion/demo.ts')).demoEnvironment(window)
    : import.meta.env.DEV
      ? (await import('./lib/companion/dev.ts')).devEnvironment(window)
      : pageEnvironment(window);

const connection = isPair ? null : connect(env);
const companion = connection?.companion ?? null;
const events = connection?.events ?? null;
const updates = createUpdatesStore(env.updates);
watchForUpdates(updates);
// A Settings change that reloaded the window comes back to the section it was made in.
if (!isPair) reopenSettings();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <WindowFrame />
    <CompanionContext value={companion}>
      <EventsContext value={events}>
        <UpdatesContext value={updates}>
          <FolderPickerContext value={env.pickFolder ?? null}>{isPair ? <Pair initialCode={initialCode} /> : <App />}</FolderPickerContext>
        </UpdatesContext>
      </EventsContext>
    </CompanionContext>
  </StrictMode>,
);
