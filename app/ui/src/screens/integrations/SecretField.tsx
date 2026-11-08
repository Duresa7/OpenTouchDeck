import { useStore } from 'zustand';
import { useKeychainName } from '../../lib/companion/keychain.ts';
import type { DetailStore } from '../../lib/integrations/detail.ts';
import type { FormField } from '../../lib/integrations/model.ts';
import { StoredField } from '../../shell/StoredField.tsx';

/** A write-only Integration secret. Once set it shows only "Set", with Replace and Forget, which wait while the switch writes. */
export function SecretField({ store, field, integration, switching = false }: { store: DetailStore; field: FormField; integration: string; switching?: boolean }) {
  const secret = useStore(store, (s) => s.secrets[field.name]);
  const error = useStore(store, (s) => s.errors[field.name]);
  const busy = useStore(store, (s) => s.busy);
  const keychain = useKeychainName();

  return (
    <StoredField
      label={field.label}
      hint={`Stored in ${keychain}, never in a file.`}
      stored={{ kind: 'secret', set: secret?.set === true, label: secret?.hint ?? undefined }}
      busy={busy !== null || switching}
      error={error}
      testid="secret"
      autoComplete="new-password"
      forget={{ title: `Forget the ${field.label.toLowerCase()}?`, body: `${integration} stops until you add it again.` }}
      onSave={(value) => store.putSecret({ field: field.name, value })}
      onForget={() => void store.forgetSecret(field.name)}
    />
  );
}
