import type {
  BaseCodingAgent,
  ExecutorConfig,
  ExecutorProfile,
  ExecutorProfileId,
} from 'shared/types';
import { getSortedExecutorVariantKeys } from '@/shared/lib/executor';

interface ResolveExecutorConfigForSelectionArgs {
  executor: BaseCodingAgent;
  profiles: Record<string, ExecutorProfile> | null;
  configExecutorProfile?: ExecutorProfileId | null;
}

export function resolveExecutorConfigForSelection({
  executor,
  profiles,
  configExecutorProfile,
}: ResolveExecutorConfigForSelectionArgs): ExecutorConfig {
  const executorProfile = profiles?.[executor];
  if (!executorProfile) {
    return { executor, variant: null };
  }

  const variants = getSortedExecutorVariantKeys(executorProfile);
  let variant: string | null = null;

  if (
    configExecutorProfile?.executor === executor &&
    configExecutorProfile.variant &&
    variants.includes(configExecutorProfile.variant)
  ) {
    variant = configExecutorProfile.variant;
  }

  if (!variant) {
    variant = variants.includes('DEFAULT') ? 'DEFAULT' : (variants[0] ?? null);
  }

  return { executor, variant };
}
