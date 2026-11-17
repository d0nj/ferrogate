import { QueryClient } from '@tanstack/react-query'

import { ApiError } from '@/api/client'

/**
 * Retries only what is worth retrying.
 *
 * A 4xx means the request itself is wrong — a missing session, a bad id, a
 * validation failure — and repeating it changes nothing while making the
 * console look slow. Server and transport failures are transient, so they get
 * two more attempts with backoff.
 */
function shouldRetry(failureCount: number, error: unknown): boolean {
  if (failureCount >= 2) return false
  if (error instanceof ApiError) return error.status >= 500
  // A thrown non-ApiError is a network or parsing fault.
  return true
}

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: shouldRetry,
        staleTime: 10_000,
        refetchOnWindowFocus: false,
      },
      mutations: {
        // Mutations are not idempotent: a retried delete or create can duplicate
        // work, so the caller decides whether to try again.
        retry: false,
      },
    },
  })
}

export const queryClient = createQueryClient()
