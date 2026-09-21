export type Mode = 'episode' | 'season';
export type DeliveryState = 'pending' | 'sending' | 'sent' | 'uncertain' | 'failed' | 'covered';
export interface Show { id: number; title: string; excluded: boolean; active: boolean; mode: Mode; mode_overridden: boolean }
export interface Health {
	worker: { last_scan_at: number | null; last_scan_succeeded: boolean; last_delivery_at: number | null };
	webhook_disabled: boolean;
	webhook_retry_at: number;
	tracking_since: number;
	unresolved_deliveries: number;
}
export interface Notification {
	awaiting_confirmation: boolean;
	key: string; series_id: number; mode: Mode; due_at: number; season: number; content: string;
	state: DeliveryState; attempted_at: number | null; sent_at: number | null;
}

export async function api<T>(path: string, options: RequestInit = {}): Promise<T> {
	const response = await fetch(`/api/${path}`, {
		...options,
		headers: { 'content-type': 'application/json', ...options.headers },
		signal: options.signal ?? AbortSignal.timeout(35_000)
	});
	if (!response.ok) {
		let message = `Request failed (${response.status}). Please retry.`;
		try {
			const body: unknown = await response.json();
			if (typeof body === 'object' && body !== null && 'error' in body && typeof body.error === 'string') message = body.error;
		} catch { /* The server may return an empty or plain-text error. */ }
		throw new Error(message);
	}
	return response.status === 204 ? undefined as T : await response.json() as T;
}
export const errorMessage = (error: unknown) => error instanceof Error ? error.message : 'The request failed. Please retry.';

export interface ProgressCounts { total: number; aired: number; in_library: number; undated: number }
export interface NextNotification {
	season: number; episode: number | null; due_at: number;
	awaiting_confirmation: boolean; episodes_remaining: number;
}
export interface SeriesProgress {
	show: Show; status: string; monitored: boolean; as_of: number; tracking_since: number;
	counts: ProgressCounts;
	notification_block: string | null;
	next_release: { season: number; episode: number; title: string; air_at: number } | null;
	next_notification: NextNotification | null;
	seasons: {
		number: number; counts: ProgressCounts; completion_confirmed: boolean; final_air_at: number | null;
		notification: NextNotification | null;
		episodes: { id: number; number: number; title: string; air_at: number | null; has_file: boolean; monitored: boolean; notified: boolean }[];
	}[];
}
