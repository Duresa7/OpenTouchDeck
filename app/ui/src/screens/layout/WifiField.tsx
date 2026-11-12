import { useEffect, useId, useState, type FormEvent } from 'react';
import { useStore } from 'zustand';
import type { DeviceSummary } from '../../lib/companion/api.ts';
import { carrierText, CODE_DIGITS, physicalAccess, type WifiStore } from '../../lib/device/wifi.ts';
import type { WifiSetupStore } from '../../lib/device/wifi-setup.ts';
import { button } from '../../shell/button.ts';
import { WifiSetupField } from './WifiSetupField.tsx';

interface WifiFieldProps {
  device: DeviceSummary;
  name: string;
  wifi: WifiStore;
  /** The network the Device joins, set over USB. */
  setup: WifiSetupStore;
  /** Connected and not restarting. */
  live: boolean;
}

type PairingProps = Pick<WifiFieldProps, 'name' | 'wifi'>;

function CodeForm({ name, wifi }: PairingProps) {
  const id = useId();
  const view = useStore(wifi, (s) => s.view);
  const problem = useStore(wifi, (s) => s.problem);
  const [code, setCode] = useState('');
  const checking = view.step === 'checking';
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!checking) void wifi.getState().submit(code);
  };
  if (view.step !== 'code' && view.step !== 'checking') return null;
  return (
    <form className="panel-confirm" onSubmit={submit} data-testid="device-panel-wifi-code-form">
      <label className="panel-confirm-title" htmlFor={id}>
        Enter the code on {name}
      </label>
      <p className="field-hint">
        {name} at {view.address} shows {CODE_DIGITS} digits.
      </p>
      <input
        id={id}
        className="input tabular"
        value={code}
        inputMode="numeric"
        autoComplete="one-time-code"
        maxLength={CODE_DIGITS}
        spellCheck={false}
        aria-describedby={problem ? undefined : `${id}-left`}
        onChange={(event) => setCode(event.target.value.replace(/\D/gu, ''))}
        data-testid="device-panel-wifi-code"
      />
      {!problem && (
        <p id={`${id}-left`} className="field-hint tabular" data-testid="device-panel-wifi-attempts">
          {view.attemptsLeft === 1 ? '1 try left' : `${view.attemptsLeft} tries left`}
        </p>
      )}
      <div className="panel-confirm-buttons">
        <button type="button" className={button({ size: 'small' })} onClick={() => void wifi.getState().cancel()} data-testid="device-panel-wifi-cancel">
          Cancel
        </button>
        <button type="submit" className={button({ kind: 'primary', size: 'small' })} aria-disabled={checking} data-testid="device-panel-wifi-pair">
          Pair
        </button>
      </div>
    </form>
  );
}

function AddressForm({ name, wifi }: PairingProps) {
  const id = useId();
  const view = useStore(wifi, (s) => s.view);
  const address = useStore(wifi, (s) => s.address);
  const starting = view.step === 'starting';
  if (view.step !== 'address' && !starting) return null;
  const found = view.step === 'address' && view.found;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!starting) void wifi.getState().start();
  };
  return (
    <form className="panel-confirm" onSubmit={submit} data-testid="device-panel-wifi-address-form">
      <label className="panel-confirm-title" htmlFor={id}>
        {name}'s address
      </label>
      <p className="field-hint" data-testid="device-panel-wifi-found">
        {found ? `Found ${name} on the network.` : `Type the address ${name} shows under Wi-Fi.`} On {name}, choose Pair new app first.
      </p>
      <p className="field-hint" data-testid="device-panel-wifi-notice">
        {physicalAccess(name)}
      </p>
      <input
        id={id}
        className="input"
        value={address}
        placeholder="IP address or name"
        spellCheck={false}
        autoComplete="off"
        onChange={(event) => wifi.getState().type(event.target.value)}
        data-testid="device-panel-wifi-address"
      />
      <div className="panel-confirm-buttons">
        <button type="button" className={button({ size: 'small' })} onClick={() => void wifi.getState().cancel()} data-testid="device-panel-wifi-cancel">
          Cancel
        </button>
        <button
          type="submit"
          className={button({ kind: 'primary', size: 'small' })}
          aria-disabled={starting || !address.trim()}
          data-testid="device-panel-wifi-start"
        >
          Start pairing
        </button>
      </div>
    </form>
  );
}

/** Everything Wi-Fi for one Device, in the order a person needs it: the network it joins (set over USB, radio
 * build only), its Pairing with this app over that network and the flow that makes one, then the Carrier it is on. */
export function WifiField({ device, name, wifi, setup, live }: WifiFieldProps) {
  const id = useId();
  const pairing = useStore(wifi, (s) => s.pairing);
  const loaded = useStore(wifi, (s) => s.loaded);
  const view = useStore(wifi, (s) => s.view);
  const problem = useStore(wifi, (s) => s.problem);
  const carrier = carrierText(device);
  useEffect(() => void wifi.getState().load(), [wifi]);

  const sub = pairing
    ? pairing.removing
      ? `Being removed from ${name}. Plug ${name} in to finish.`
      : (pairing.problem ?? `Paired at ${pairing.address}.`)
    : view.step === 'finding'
      ? `Looking for ${name} on the network…`
      : 'Not paired. Pair to use it without the cable.';
  return (
    <div className="field" data-testid="device-panel-wifi" data-carrier={device.carrier ?? undefined}>
      <span className="field-label" id={id}>
        Wi-Fi
      </span>
      <WifiSetupField device={device} name={name} setup={setup} live={live} />
      <div className="panel-row" aria-labelledby={id} role="group">
        <span className="row-text">
          <span className="row-title">Pairing</span>
          <span className="row-sub" data-testid="device-panel-wifi-status" aria-live="polite">
            {sub}
          </span>
        </span>
        <span className="panel-row-buttons">
          {pairing ? (
            <button type="button" className={button({ size: 'small' })} onClick={() => void wifi.getState().forget()} data-testid="device-panel-wifi-forget">
              Forget
            </button>
          ) : (
            view.step === 'idle' && (
              <button
                type="button"
                className={button({ size: 'small' })}
                aria-disabled={loaded !== 'ready'}
                onClick={() => loaded === 'ready' && void wifi.getState().begin()}
                data-testid="device-panel-wifi-begin"
              >
                Pair over Wi-Fi
              </button>
            )
          )}
        </span>
      </div>
      <AddressForm name={name} wifi={wifi} />
      <CodeForm name={name} wifi={wifi} />
      {problem && (
        <p className="field-error" role="alert" data-testid="device-panel-wifi-problem">
          {problem}
        </p>
      )}
      {carrier && (
        <p className="field-hint" data-testid="device-panel-carrier">
          {carrier}
        </p>
      )}
    </div>
  );
}
