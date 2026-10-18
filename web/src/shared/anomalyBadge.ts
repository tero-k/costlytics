/**
 * The sidebar's open-anomaly count. Polls `getAnomalies()` every 60 s while
 * a page is open (checks run in the backend; there's no push channel).
 * Failures leave the badge as it was.
 */

import { getAnomalies } from '../api.ts';
import { badgeText } from './anomalies.ts';

const POLL_MS = 60_000;

/** Sets the badge from `count`, or fetches it when omitted. */
export async function refreshAnomalyBadge(count?: number): Promise<void> {
  const badge = document.getElementById('nav-badge-anomalies');
  if (!badge) return;
  try {
    const n = count ?? (await getAnomalies()).open_count;
    badge.textContent = badgeText(n);
    badge.hidden = n <= 0;
  } catch {
    // Backend not ready or unreachable: keep the last value.
  }
}

export function initAnomalyBadge(): void {
  void refreshAnomalyBadge();
  window.setInterval(() => void refreshAnomalyBadge(), POLL_MS);
}
