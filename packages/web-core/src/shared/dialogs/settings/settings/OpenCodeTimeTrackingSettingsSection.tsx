import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  CheckIcon,
  CopyIcon,
  SpinnerIcon,
  TrashIcon,
} from '@phosphor-icons/react';
import type { OpenCodeTimeTrackingTokenSummary } from 'shared/types';
import { PrimaryButton } from '@vibe/ui/components/PrimaryButton';
import { opencodeTimeTrackingApi } from '@/shared/lib/api';
import { cn } from '@/shared/lib/utils';
import {
  SettingsCard,
  SettingsField,
  SettingsInput,
} from './SettingsComponents';
import { useSettingsHost } from './SettingsHostContext';

const DEFAULT_LOCAL_ORIGIN = 'http://127.0.0.1:9000';
const TOKEN_PLACEHOLDER = 'vktt_...';

function getSnippetOrigin() {
  if (typeof window === 'undefined') {
    return DEFAULT_LOCAL_ORIGIN;
  }

  return window.location.origin || DEFAULT_LOCAL_ORIGIN;
}

function formatDate(value: string | null, fallback: string) {
  if (!value) {
    return fallback;
  }

  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(value));
}

function buildOpenCodeConfigSnippet(origin: string, token: string) {
  return JSON.stringify(
    {
      plugin: [
        [
          '@vibe/opencode-time-tracker',
          {
            servers: {
              [origin]: {
                token,
              },
            },
          },
        ],
      ],
    },
    null,
    2
  );
}

export function OpenCodeTimeTrackingSettingsSection() {
  const { t } = useTranslation('settings');
  const { selectedHost } = useSettingsHost();
  const hostId = selectedHost?.apiHostId ?? null;
  const [tokens, setTokens] = useState<OpenCodeTimeTrackingTokenSummary[]>([]);
  const [label, setLabel] = useState('');
  const [createdToken, setCreatedToken] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [creating, setCreating] = useState(false);
  const [revokingId, setRevokingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copyStatus, setCopyStatus] = useState<'idle' | 'success' | 'error'>(
    'idle'
  );

  const snippetOrigin = useMemo(getSnippetOrigin, []);
  const configSnippet = useMemo(
    () =>
      buildOpenCodeConfigSnippet(
        snippetOrigin,
        createdToken ?? TOKEN_PLACEHOLDER
      ),
    [createdToken, snippetOrigin]
  );
  const activeTokens = useMemo(
    () => tokens.filter((token) => !token.revoked_at),
    [tokens]
  );
  const neverLabel = t('settings.opencodeTimeTracking.tokens.never', 'Never');

  const loadTokens = async () => {
    setLoading(true);
    setError(null);

    try {
      setTokens(await opencodeTimeTrackingApi.listTokens(hostId));
    } catch (err) {
      setError(
        err instanceof Error
          ? err.message
          : t(
              'settings.opencodeTimeTracking.errors.loadFailed',
              'Failed to load OpenCode plugin tokens.'
            )
      );
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void loadTokens();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hostId]);

  const handleCreateToken = async () => {
    setCreating(true);
    setError(null);
    setCreatedToken(null);
    setCopyStatus('idle');

    try {
      const trimmedLabel = label.trim();
      const response = await opencodeTimeTrackingApi.createToken(
        trimmedLabel ? { label: trimmedLabel } : {},
        hostId
      );
      setCreatedToken(response.token);
      setLabel('');
      await loadTokens();
    } catch (err) {
      setError(
        err instanceof Error
          ? err.message
          : t(
              'settings.opencodeTimeTracking.errors.createFailed',
              'Failed to create OpenCode plugin token.'
            )
      );
    } finally {
      setCreating(false);
    }
  };

  const handleRevokeToken = async (tokenId: string) => {
    setRevokingId(tokenId);
    setError(null);

    try {
      await opencodeTimeTrackingApi.revokeToken(tokenId, hostId);
      setTokens((current) => current.filter((token) => token.id !== tokenId));
    } catch (err) {
      setError(
        err instanceof Error
          ? err.message
          : t(
              'settings.opencodeTimeTracking.errors.revokeFailed',
              'Failed to revoke OpenCode plugin token.'
            )
      );
    } finally {
      setRevokingId(null);
    }
  };

  const handleCopySnippet = async () => {
    setCopyStatus('idle');

    try {
      if (!navigator.clipboard?.writeText) {
        throw new Error('Clipboard API unavailable');
      }

      await navigator.clipboard.writeText(configSnippet);
      setCopyStatus('success');
      setTimeout(() => setCopyStatus('idle'), 2000);
    } catch {
      setCopyStatus('error');
    }
  };

  return (
    <>
      {error && (
        <div className="bg-error/10 border border-error/50 rounded-sm p-4 text-error text-sm">
          {error}
        </div>
      )}

      <SettingsCard
        title={t(
          'settings.opencodeTimeTracking.title',
          'OpenCode time tracking'
        )}
        description={t(
          'settings.opencodeTimeTracking.description',
          'Create local plugin tokens that let the OpenCode time-tracking plugin submit active-time entries to this Vibe Kanban host.'
        )}
      >
        <div className="rounded-sm border border-warning/50 bg-warning/10 p-3 text-sm text-warning">
          {t(
            'settings.opencodeTimeTracking.restartReminder',
            'Restart OpenCode after installing the plugin or changing its config.'
          )}
        </div>

        <SettingsField
          label={t(
            'settings.opencodeTimeTracking.create.label',
            'New token label'
          )}
          description={t(
            'settings.opencodeTimeTracking.create.helper',
            'Labels are optional and only help you identify tokens later.'
          )}
        >
          <div className="flex gap-2">
            <SettingsInput
              id="opencode-time-tracking-token-label"
              aria-label={t(
                'settings.opencodeTimeTracking.create.label',
                'New token label'
              )}
              value={label}
              onChange={setLabel}
              placeholder={t(
                'settings.opencodeTimeTracking.create.placeholder',
                'OpenCode on this machine'
              )}
              disabled={creating}
            />
            <PrimaryButton
              value={t(
                'settings.opencodeTimeTracking.create.button',
                'Create token'
              )}
              onClick={handleCreateToken}
              disabled={creating}
              actionIcon={creating ? 'spinner' : undefined}
              className="shrink-0"
            />
          </div>
        </SettingsField>

        {createdToken && (
          <div className="space-y-3 rounded-sm border border-success/50 bg-success/10 p-3">
            <div>
              <p className="text-sm font-medium text-success">
                {t(
                  'settings.opencodeTimeTracking.created.title',
                  'Token created'
                )}
              </p>
              <p className="mt-1 text-sm text-low">
                {t(
                  'settings.opencodeTimeTracking.created.warning',
                  'Copy this token now. It is shown only once and cannot be recovered later.'
                )}
              </p>
            </div>
            <div className="rounded-sm border border-border bg-secondary px-base py-half font-mono text-sm text-high break-all select-all">
              {createdToken}
            </div>
          </div>
        )}
      </SettingsCard>

      <SettingsCard
        title={t(
          'settings.opencodeTimeTracking.install.title',
          'OpenCode plugin config'
        )}
        description={t(
          'settings.opencodeTimeTracking.install.description',
          'Install @vibe/opencode-time-tracker and add this snippet to your OpenCode config. Use the newly created token in place of the placeholder.'
        )}
        headerAction={
          <PrimaryButton
            variant="tertiary"
            value={t('settings.opencodeTimeTracking.install.copy', 'Copy')}
            onClick={handleCopySnippet}
          >
            {copyStatus === 'success' ? (
              <CheckIcon
                aria-hidden="true"
                className="size-icon-xs text-success"
                weight="bold"
              />
            ) : (
              <CopyIcon
                aria-hidden="true"
                className="size-icon-xs"
                weight="bold"
              />
            )}
          </PrimaryButton>
        }
      >
        <pre className="overflow-x-auto rounded-sm border border-border bg-secondary p-3 text-xs text-high">
          <code>{configSnippet}</code>
        </pre>
        {copyStatus === 'error' && (
          <p className="text-sm text-warning">
            {t(
              'settings.opencodeTimeTracking.install.copyFailed',
              'Could not copy automatically. Select and copy the snippet manually.'
            )}
          </p>
        )}
        <p className="text-sm text-low">
          {t(
            'settings.opencodeTimeTracking.install.restart',
            'OpenCode must be restarted after plugin or config changes before tracking starts.'
          )}
        </p>
      </SettingsCard>

      <SettingsCard
        title={t(
          'settings.opencodeTimeTracking.tokens.title',
          'Existing plugin tokens'
        )}
        description={t(
          'settings.opencodeTimeTracking.tokens.description',
          'Only token metadata is listed here. Raw tokens and token hashes are never shown after creation.'
        )}
      >
        {loading ? (
          <div className="flex items-center gap-2 py-half text-sm text-low">
            <SpinnerIcon className="size-icon-xs animate-spin" weight="bold" />
            {t(
              'settings.opencodeTimeTracking.tokens.loading',
              'Loading tokens...'
            )}
          </div>
        ) : activeTokens.length === 0 ? (
          <div className="rounded-sm border border-border bg-secondary/30 p-4 text-sm text-low">
            {t(
              'settings.opencodeTimeTracking.tokens.empty',
              'No active OpenCode plugin tokens yet.'
            )}
          </div>
        ) : (
          <div className="divide-y divide-border rounded-sm border border-border">
            {activeTokens.map((token) => (
              <div
                key={token.id}
                className="flex items-start justify-between gap-3 p-3"
              >
                <div className="min-w-0 space-y-1">
                  <div className="text-sm font-medium text-normal truncate">
                    {token.label ||
                      t(
                        'settings.opencodeTimeTracking.tokens.unlabeled',
                        'Unlabelled token'
                      )}
                  </div>
                  <div className="text-xs text-low">
                    {t(
                      'settings.opencodeTimeTracking.tokens.createdAt',
                      'Created {{date}}',
                      { date: formatDate(token.created_at, neverLabel) }
                    )}
                    {' · '}
                    {t(
                      'settings.opencodeTimeTracking.tokens.lastUsedAt',
                      'Last used {{date}}',
                      { date: formatDate(token.last_used_at, neverLabel) }
                    )}
                  </div>
                </div>
                <button
                  type="button"
                  onClick={() => void handleRevokeToken(token.id)}
                  disabled={revokingId === token.id}
                  className={cn(
                    'shrink-0 rounded-sm px-2 py-1 text-xs text-error transition-colors',
                    'hover:bg-error/10 disabled:cursor-not-allowed disabled:opacity-50'
                  )}
                >
                  {revokingId === token.id ? (
                    <span className="inline-flex items-center gap-1">
                      <SpinnerIcon
                        aria-hidden="true"
                        className="size-icon-xs animate-spin"
                        weight="bold"
                      />
                      {t(
                        'settings.opencodeTimeTracking.tokens.revoking',
                        'Revoking…'
                      )}
                    </span>
                  ) : (
                    <span className="inline-flex items-center gap-1">
                      <TrashIcon
                        aria-hidden="true"
                        className="size-icon-xs"
                        weight="bold"
                      />
                      {t(
                        'settings.opencodeTimeTracking.tokens.revoke',
                        'Revoke'
                      )}
                    </span>
                  )}
                </button>
              </div>
            ))}
          </div>
        )}
      </SettingsCard>
    </>
  );
}
