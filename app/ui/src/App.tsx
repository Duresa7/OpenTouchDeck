import { lazy, Suspense, useEffect, useLayoutEffect } from 'react';
import { CircleAlert, CircleCheck, Info, TriangleAlert } from 'lucide-react';
import { MotionConfig } from 'motion/react';
import { Toaster, type ToasterProps } from 'sonner';
import { useDevices } from './lib/companion/devices.ts';
import { Home } from './screens/Home.tsx';
import { SetupHost } from './screens/onboarding/SetupHost.tsx';
import { NavRail } from './shell/NavRail.tsx';
import { applyTheme } from './state/theme.ts';
import { useUi } from './state/ui.ts';

const Layout = lazy(() => import('./screens/Layout.tsx').then((screen) => ({ default: screen.Layout })));
const Integrations = lazy(() => import('./screens/Integrations.tsx').then((screen) => ({ default: screen.Integrations })));
const Settings = lazy(() => import('./screens/Settings.tsx').then((screen) => ({ default: screen.Settings })));

const TOAST_ICONS: ToasterProps['icons'] = {
  success: <CircleCheck size={16} strokeWidth={2} aria-hidden />,
  error: <CircleAlert size={16} strokeWidth={2} aria-hidden />,
  warning: <TriangleAlert size={16} strokeWidth={2} aria-hidden />,
  info: <Info size={16} strokeWidth={2} aria-hidden />,
};

const TOAST_OPTIONS: ToasterProps['toastOptions'] = {
  unstyled: true,
  classNames: {
    toast: 'toast',
    icon: 'toast-icon',
    content: 'toast-content',
    title: 'toast-title',
    description: 'toast-description',
    actionButton: 'button button-small',
    cancelButton: 'button button-small',
    success: 'is-success',
    error: 'is-error',
    warning: 'is-warning',
    info: 'is-info',
  },
};

/**
 * The app shell: the nav rail down the left edge, the current screen to its
 * right and the toast outlet. The rail comes first, so Tab reaches it before
 * the screen. Switching screens never animates. Motion follows the system's reduced
 * motion setting everywhere; the shell's own transitions are CSS and tokens.css
 * handles them.
 */
export function App() {
  const status = useDevices(s => s.status);
  useEffect(() => {
    if (status === 'unpaired' && !window.openTouchDeck) location.replace('/pair');
  }, [status]);
  const section = useUi((s) => s.section);
  const appearance = useUi((s) => s.appearance);

  useLayoutEffect(() => applyTheme(document.documentElement, appearance), [appearance]);

  return (
    <MotionConfig reducedMotion="user">
      <div className="shell">
        <NavRail />
        <div className="shell-screen">
          {section === 'home' && <Home />}
          {section === 'layout' && (
            <Suspense fallback={null}>
              <Layout />
            </Suspense>
          )}
          {section === 'integrations' && (
            <Suspense fallback={null}>
              <Integrations />
            </Suspense>
          )}
          {section === 'settings' && (
            <Suspense fallback={null}>
              <Settings />
            </Suspense>
          )}
        </div>
      </div>
      <SetupHost />
      <Toaster theme={appearance} position="bottom-center" icons={TOAST_ICONS} toastOptions={TOAST_OPTIONS} />
    </MotionConfig>
  );
}
