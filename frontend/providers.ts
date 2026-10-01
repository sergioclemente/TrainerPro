import type { AppError, PlanSyncReport, ProviderConnection } from "./ipc";

export type PlanRefreshOutcome = {
  connection: ProviderConnection;
} & ({ report: PlanSyncReport; error?: never } | { error: AppError; report?: never });

/** Refresh each selected plan source independently. A failed or unauthenticated
 * source cannot suppress another source's refresh or invalidate cached plans. */
export async function refreshPlanConnections(
  connections: ProviderConnection[],
  refresh: (connectionId: string) => Promise<PlanSyncReport>,
): Promise<PlanRefreshOutcome[]> {
  const outcomes: PlanRefreshOutcome[] = [];
  for (const connection of connections) {
    if (!connection.provider.capabilities.includes("planning") || !connection.connection_id) continue;
    if (connection.state !== "connected") {
      outcomes.push({ connection, error: connection.error ?? {
        code: "provider_authentication", message: "Sign in again in Settings → Connections to sync plans.",
      } });
      continue;
    }
    try { outcomes.push({ connection, report: await refresh(connection.connection_id) }); }
    catch (failure) {
      const error = failure as AppError;
      outcomes.push({ connection, error: { code: error.code ?? "provider_sync", message: error.message ?? String(failure) } });
    }
  }
  return outcomes;
}
