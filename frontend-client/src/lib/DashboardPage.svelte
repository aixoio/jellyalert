<script lang="ts">
	import { onMount } from 'svelte';
	import { api, errorMessage, colorHex, type EmbedColors, type Health, type Notification, type Show, type DeliveryState } from '$lib/api';
	import Time from '$lib/Time.svelte';
	import ShowPoster from '$lib/ShowPoster.svelte';
	import EmbedPreview from '$lib/EmbedPreview.svelte';

	let { section }: { section: 'overview' | 'shows' | 'activity' } = $props();
	const titles = { overview: 'Overview', shows: 'Shows', activity: 'Activity' };
	const descriptions = { overview: 'Check your connection and keep Discord alerts running.', shows: 'Choose what to track and when each show should notify you.', activity: 'Follow upcoming alerts and review delivery history.' };

	let colors = $state<EmbedColors>({ episode_color: 0x5865f2, season_color: 0xf1c40f });
	let health = $state<Health | null>(null);
	let shows = $state<Show[]>([]);
	let notifications = $state<Notification[]>([]);
	let loading = $state(true);
	let refreshing = $state(false);
	let busy = $state('');
	let error = $state('');
	let toast = $state<{ ok: boolean; message: string } | null>(null);
	let toastTimer: ReturnType<typeof setTimeout> | undefined;

	function dismissToast() {
		clearTimeout(toastTimer);
		toast = null;
	}

	function showToast(message: string, ok = true) {
		if (!alive) return;
		dismissToast();
		toast = { ok, message };
		toastTimer = setTimeout(dismissToast, ok ? 5000 : 10000);
	}
	let updatedAt = $state<number | null>(null);
	let query = $state('');
	let filter = $state('active');
	let offset = $state(0);
	let showPage = $state(0);
	let previewDialog = $state<HTMLDialogElement>();
	let selectedNotification = $state<Notification | null>(null);
	let activityView = $state<'upcoming' | 'history'>('upcoming');
	let hasNext = $state(false);
	let alive = true;
	const pageSize = 20;
	let locked = $derived(loading || refreshing || busy !== '');
	let tracked = $derived(shows.filter((show) => show.active && !show.excluded).length);
	let excluded = $derived(shows.filter((show) => show.active && show.excluded).length);
	let visibleShows = $derived(shows.filter((show) => {
		const matches = show.title.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase());
		return matches && (filter === 'all' || (filter === 'active' && show.active) || (filter === 'tracked' && show.active && !show.excluded) || (filter === 'excluded' && show.excluded) || (filter === 'removed' && !show.active));
	}).sort((a, b) => a.title.localeCompare(b.title)));
	let showPageCount = $derived(Math.ceil(visibleShows.length / 12));
	let paginationStart = $derived(Math.max(0, Math.min(showPage - 2, showPageCount - 5)));
	let showPages = $derived(Array.from({ length: Math.min(5, showPageCount) }, (_, index) => paginationStart + index));
	let pagedShows = $derived(visibleShows.slice(showPage * 12, (showPage + 1) * 12));
	$effect(() => { query; filter; showPage = 0; });
	$effect(() => {
		const lastPage = Math.max(0, Math.ceil(visibleShows.length / 12) - 1);
		if (showPage > lastPage) showPage = lastPage;
	});
	const stateLabels: Record<DeliveryState, string> = { pending: 'Scheduled', sending: 'Attempt recorded', sent: 'Sent', uncertain: 'Unconfirmed', failed: 'Failed', covered: 'Already notified' };
	const stateClasses: Record<DeliveryState, string> = { pending: 'badge-info', sending: 'badge-warning', sent: 'badge-success', uncertain: 'badge-warning', failed: 'badge-error', covered: 'badge-ghost' };

	async function refresh(pageOffset = offset, view = activityView) {
		if (refreshing || busy) return;
		refreshing = true;
		try {
			const [nextHealth, nextShows, nextNotifications, nextColors] = await Promise.all([
				section === 'overview' ? api<Health>('health') : Promise.resolve(null),
				section !== 'overview' ? api<Show[]>('shows') : Promise.resolve([]),
				section === 'activity' ? api<Notification[]>(`notifications?limit=${pageSize + 1}&offset=${pageOffset}&view=${view}`) : Promise.resolve([]),
				section === 'activity' ? api<EmbedColors>('settings/colors') : Promise.resolve(colors)
			]);
			if (!alive) return;
			health = nextHealth;
			colors = nextColors;
			shows = nextShows;
			notifications = nextNotifications.slice(0, pageSize);
			hasNext = nextNotifications.length > pageSize;
			offset = pageOffset;
			activityView = view;
			updatedAt = Math.floor(Date.now() / 1000);
			error = '';
		} catch (cause) {
			if (alive) error = errorMessage(cause);
		} finally {
			if (alive) { loading = false; refreshing = false; }
		}
	}

	async function change(name: string, path: string, method: 'PUT' | 'POST', body: unknown, success: string, saved?: () => void) {
		if (locked) return;
		busy = name;
		dismissToast();
		error = '';
		try {
			await api<void>(path, { method, body: body === undefined ? undefined : JSON.stringify(body) });
			if (name.startsWith('show:')) {
				const id = Number(name.slice(5));
				shows = shows.map((show) => show.id === id ? { ...show, excluded: !show.excluded } : show);
			}
			saved?.();
			showToast(success);
			busy = '';
			await refresh(0);
		} catch (cause) {
			error = `${errorMessage(cause)} Refresh to confirm the current setting before retrying.`;
		} finally { busy = ''; }
	}

	async function sendTest(notification: Notification) {
		if (locked) return;
		busy = `test:${notification.key}`;
		try {
			await api<void>(`notifications/${encodeURIComponent(notification.key)}/test`, { method: 'POST' });
			showToast('Test sent to Discord. Normal delivery is unchanged.');
		} catch (cause) {
			showToast(`${errorMessage(cause)} Normal delivery is unchanged.`, false);
		} finally { busy = ''; }
	}

	onMount(() => {
		alive = true;
		void refresh();
		const timer = setInterval(() => { if (!document.hidden) void refresh(); }, 30_000);
		const visible = () => { if (!document.hidden) void refresh(); };
		document.addEventListener('visibilitychange', visible);
		return () => { alive = false; clearTimeout(toastTimer); clearInterval(timer); document.removeEventListener('visibilitychange', visible); };
	});
</script>

<svelte:head>
	<title>{titles[section]} · Jelly Alert</title>
	<meta name="description" content="Control episode and season alerts, choose which shows to track, and follow Discord deliveries." />
</svelte:head>

<div>
    <header class="flex flex-wrap items-center justify-between gap-4 mb-6 md:mb-8">
        <div class="space-y-2"><h1 class="card-title">{titles[section]}</h1><p>{descriptions[section]}</p></div>
		<button class="btn btn-outline" onclick={() => refresh()} disabled={locked}>
			{#if refreshing}<span class="loading loading-spinner loading-xs" aria-hidden="true"></span>{/if}
			{refreshing ? 'Refreshing' : 'Refresh'}
		</button>
	</header>

	{#if error}
		<div role="alert" class="alert alert-error mb-6">
			<div><strong>{updatedAt ? 'Unable to update' : 'Unable to connect'}</strong><p>{error}</p>{#if updatedAt}<p>Showing the last successful refresh. <Time value={updatedAt} /></p>{/if}</div>
			<button class="btn btn-sm" onclick={() => refresh()} disabled={locked}>Retry</button>
		</div>
	{/if}
	<div class="toast toast-end toast-bottom z-50 max-w-full" role="status" aria-live="polite" aria-atomic="true">
		{#if toast && !selectedNotification}
			<div class="alert max-w-sm" class:alert-success={toast.ok} class:alert-warning={!toast.ok}>
				<span>{toast.message}</span>
				<button class="btn btn-ghost btn-sm" onclick={dismissToast} aria-label="Dismiss notification">✕</button>
			</div>
		{/if}
	</div>

	{#if loading}
		<div class="grid justify-items-center gap-3 py-10 text-center" role="status"><span class="loading loading-spinner loading-lg"></span><p>Connecting to Server Core…</p></div>
	{:else if updatedAt}
		<div class="grid min-w-0 grid-cols-1 gap-6">
			{#if section === 'overview' && health}
			<div class="grid min-w-0 grid-cols-1 items-start gap-6 xl:grid-cols-2">
                <section class="card card-border">
                    <div class="card-body">
                        <h2 class="card-title">Your shows, your schedule</h2>
                        <p>Choose every episode or full seasons on the Shows page. Changes save immediately and only affect that show.</p>
                        <p><strong>Every episode:</strong> an alert at each missing episode’s air time.</p>
                        <p><strong>Full seasons:</strong> one alert when the complete season has aired and episodes are missing. Sonarr must confirm season completion.</p>
                        <p>New shows start with episode alerts. Turn tracking off to exclude a show entirely.</p>
                        <p>Air times come from Sonarr and do not confirm that a download is available.</p>
                        <div class="card-actions"><a class="btn btn-primary" href="/shows">Manage shows</a><a class="btn btn-outline" href="/activity">View activity</a></div>
                    </div>
                </section>

				<aside id="status" class="card card-border" aria-labelledby="status-heading">
					<div class="card-body">
						<div class="flex flex-wrap items-center justify-between gap-4"><h2 id="status-heading" class="card-title">Service status</h2><span class="badge" class:badge-success={!error} class:badge-warning={!!error}>{error ? 'Stale' : 'Connected'}</span></div>
						<dl class="divide-y divide-base-300 [&>div]:flex [&>div]:flex-wrap [&>div]:justify-between [&>div]:gap-3 [&>div]:py-3">
							<div><dt>Sonarr scan</dt><dd><span class="badge" class:badge-success={health.worker.last_scan_succeeded} class:badge-warning={!health.worker.last_scan_succeeded}>{health.worker.last_scan_at === null ? 'Waiting for first scan' : health.worker.last_scan_succeeded ? 'Healthy' : 'Retrying'}</span></dd></div>
							<div><dt>Last scan</dt><dd><Time value={health.worker.last_scan_at} /></dd></div>
							<div><dt>Discord delivery</dt><dd><span class="badge" class:badge-error={health.webhook_disabled} class:badge-warning={!health.webhook_disabled && health.webhook_retry_at > (updatedAt ?? 0)} class:badge-success={!health.webhook_disabled && health.webhook_retry_at <= (updatedAt ?? 0)}>{health.webhook_disabled ? 'Paused' : health.webhook_retry_at > (updatedAt ?? 0) ? 'Waiting to retry' : 'Ready'}</span></dd></div>
							<div><dt>Last successful delivery</dt><dd><Time value={health.worker.last_delivery_at} /></dd></div>
							<div><dt>Tracking since</dt><dd><Time value={health.tracking_since} /></dd></div>
						</dl>
						{#if health.webhook_disabled}<div class="alert alert-warning"><p>Discord rejected the webhook. Update the webhook in Server Core’s configuration and restart it, then resume notifications here.</p></div><button class="btn btn-outline" disabled={locked} onclick={() => change('resume', 'webhook/resume', 'POST', undefined, 'Discord notifications resumed.')}>{busy === 'resume' ? 'Resuming…' : 'Resume notifications'}</button>{/if}
						{#if !health.webhook_disabled && health.webhook_retry_at > (updatedAt ?? 0)}<p>Automatic retry after <Time value={health.webhook_retry_at} />.</p>{/if}
						{#if health.unresolved_deliveries > 0}<div class="alert alert-warning"><span>{health.unresolved_deliveries} delivery attempt{health.unresolved_deliveries === 1 ? ' needs' : 's need'} attention. <a class="link" href="/activity">Review delivery activity</a>. Unconfirmed attempts are not retried to avoid duplicates.</span></div>{/if}
					</div>
				</aside>
			</div>

            {/if}
            {#if section === 'shows'}
			<section id="shows" class="grid min-w-0 grid-cols-1 gap-6" aria-label="Show preferences">
				<div class="grid min-w-0 grid-cols-1 gap-6">
					<div class="flex flex-wrap items-center justify-between gap-4"><p>Changes save automatically. Manage the shared default in Settings.</p><div class="flex flex-wrap items-center gap-4"><span class="badge badge-soft">{tracked} tracked</span><span class="badge badge-ghost">{excluded} excluded</span></div></div>
                    <div class="card card-border"><div class="card-body grid grid-cols-1 items-end gap-4 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto]">
                        <label class="fieldset min-w-0"><span class="fieldset-legend">Search shows</span><input class="input w-full" type="search" placeholder="Search by title…" bind:value={query} /></label>
                        <label class="fieldset min-w-0"><span class="fieldset-legend">Show status</span><select class="select w-full" aria-label="Filter shows" bind:value={filter}><option value="active">In Sonarr</option><option value="tracked">Tracked</option><option value="excluded">Excluded</option><option value="removed">Removed from Sonarr</option><option value="all">All shows</option></select></label>
                        <button class="btn btn-ghost" disabled={!query && filter === 'active'} onclick={() => { query = ''; filter = 'active'; }}>Clear filters</button>
                    </div></div>
					{#if visibleShows.length}
						<div class="grid grid-cols-1 gap-4 xl:grid-cols-2">
                            {#each pagedShows as show (show.id)}
                                <article class="card card-side card-border card-sm min-w-0" aria-label={show.title}>
                                    <a href={`/shows/${show.id}`} aria-label={`View ${show.title} progress`} class="relative block w-1/3 max-w-32 shrink-0 self-stretch overflow-hidden rounded-l-box"><ShowPoster id={show.id} fullWidth /></a>
                                    <div class="card-body min-w-0">
                                        <div class="flex flex-wrap items-center justify-between gap-4"><span class="badge" class:badge-ghost={!show.active || show.excluded} class:badge-success={show.active && !show.excluded} class:badge-soft={show.active && !show.excluded}>{!show.active ? 'Removed from Sonarr' : show.excluded ? 'Excluded' : 'Tracked'}</span>{#if busy === `show:${show.id}` || busy === `mode:${show.id}`}<span class="loading loading-spinner loading-xs" aria-label="Saving"></span>{/if}</div>
                                        <h2 class="card-title wrap-anywhere"><a class="link link-hover" href={`/shows/${show.id}`} aria-label={`View ${show.title} progress`}>{show.title}</a></h2>
                                        <div class="mt-auto grid gap-3">
                                            <label class="fieldset"><span class="fieldset-legend">Notify me</span><select class="select w-full" aria-label={`Notification mode for ${show.title}`} value={show.mode_overridden ? show.mode : 'default'} disabled={locked || !show.active}
                                        onchange={(event) => {
                                            const mode = event.currentTarget.value;
                                            event.currentTarget.value = show.mode_overridden ? show.mode : 'default';
                                            if (mode !== 'episode' && mode !== 'season' && mode !== 'default') return;
                                            void change(`mode:${show.id}`, `shows/${show.id}/mode`, 'PUT', { mode }, `${show.title}: ${mode === 'default' ? 'use default' : mode === 'episode' ? 'every episode' : 'full seasons'} saved.`);
                                        }}>
                                        <option value="default">Use default{!show.mode_overridden ? ` · ${show.mode === 'episode' ? 'Every episode' : 'Full seasons'}` : ''}</option>
                                        <option value="episode">Every episode</option>
                                        <option value="season">Full seasons</option>
                                    </select></label>
                                            <label class="flex items-center justify-between gap-4"><span>Track this show</span><input type="checkbox" class="toggle" aria-label={`Track ${show.title}`} checked={!show.excluded} disabled={locked || !show.active} onchange={(event) => { event.currentTarget.checked = !show.excluded; void change(`show:${show.id}`, `shows/${show.id}/exclusion`, 'PUT', { excluded: !show.excluded }, `${show.title} ${show.excluded ? 'is now tracked' : 'is now excluded'}.`); }} /></label>
                                            <p>{!show.active ? 'Restore this show in Sonarr to resume tracking.' : show.excluded ? 'Notifications are off. Your preference is kept.' : show.mode === 'season' ? 'One alert when a full season has aired.' : 'An alert when each missing episode airs.'}</p>
                                        </div>
                                    </div>
                                </article>
                            {/each}
                        </div>
					{:else}<div class="grid justify-items-center gap-3 py-10 text-center"><h3 class="card-title">{shows.length ? 'No matching shows' : 'Your shows will appear here'}</h3><p>{shows.length ? 'Try another search or filter.' : 'Add and monitor shows in Sonarr. Jelly Alert imports them on its next scan.'}</p>{#if query || filter !== 'active'}<button class="btn btn-ghost btn-sm" onclick={() => { query = ''; filter = 'active'; }}>Clear filters</button>{/if}</div>{/if}
					<div class="flex flex-wrap items-center justify-between gap-4"><span>{visibleShows.length ? `Showing ${showPage * 12 + 1}–${Math.min((showPage + 1) * 12, visibleShows.length)} of ${visibleShows.length}` : 'No shows'}</span>{#if showPageCount > 1}
                            <nav aria-label="Show pages" class="space-y-2">
                                <div class="join">
                                    <button class="btn btn-sm join-item" disabled={showPage === 0} onclick={() => showPage--} aria-label="Previous shows">«</button>
                                    {#each showPages as index (index)}
                                        <button class="btn btn-sm join-item" class:btn-active={showPage === index} aria-current={showPage === index ? 'page' : undefined} aria-label={`Show page ${index + 1}`} onclick={() => showPage = index}>{index + 1}</button>
                                    {/each}
                                    <button class="btn btn-sm join-item" disabled={showPage === showPageCount - 1} onclick={() => showPage++} aria-label="Next shows">»</button>
                                </div>
                                <p class="text-sm text-base-content/70">Page {showPage + 1} of {showPageCount}</p>
                            </nav>
                        {/if}</div>
					<p>Sonarr’s monitoring and library status also determine which episodes are eligible. Existing notification history is kept when you exclude a show.</p>
				</div>
			</section>

            {/if}
            {#if section === 'activity'}
                <section id="activity" class="space-y-4" aria-label="Notification activity" aria-busy={refreshing}>
                    <div class="flex flex-wrap items-center justify-between gap-3">
                        <div class="tabs tabs-border tabs-sm" aria-label="Activity view">
                            <button class="tab" class:tab-active={activityView === 'upcoming'} aria-pressed={activityView === 'upcoming'} disabled={locked} onclick={() => refresh(0, 'upcoming')}>Upcoming</button>
                            <button class="tab" class:tab-active={activityView === 'history'} aria-pressed={activityView === 'history'} disabled={locked} onclick={() => refresh(0, 'history')}>Delivery history</button>
                        </div>
                        <span class="text-xs text-base-content/60">{activityView === 'upcoming' ? 'Next air time first' : 'Latest air time first'}</span>
                    </div>
                    <div class="card card-border bg-base-100">
                        {#if notifications.length}
                            <div class="overflow-x-auto">
                                <table class="table table-sm">
                                    <caption class="sr-only">{activityView === 'upcoming' ? 'Upcoming notifications, next air time first' : 'Delivery history, latest air time first'}</caption>
                                    <thead class="bg-base-200"><tr><th scope="col">Show / alert</th><th scope="col" class="hidden sm:table-cell">Air time · local</th><th scope="col">Status</th><th scope="col"><span class="sr-only">Details</span></th></tr></thead>
                                    <tbody>
                                        {#each notifications as notification (notification.key)}
                                            {@const title = shows.find((show) => show.id === notification.series_id)?.title ?? `Show ${notification.series_id}`}
                                            <tr class="cursor-pointer hover:bg-base-200/50" onclick={(event) => {
                                                if ((event.target as Element).closest('a, button')) return;
                                                selectedNotification = notification;
                                                previewDialog?.showModal();
                                            }}>
                                                <td class="max-w-48 py-3 sm:max-w-sm">
                                                    <a class="link link-hover block truncate font-semibold" href={`/shows/${notification.series_id}`} title={title}>{title}</a>
                                                    <p class="text-xs text-base-content/60">Season {notification.season} · {notification.mode === 'season' ? 'Full season' : 'Episode'}</p>
                                                    <p class="mt-1 text-xs text-base-content/60 sm:hidden"><Time value={notification.due_at} /></p>
                                                </td>
                                                <td class="hidden whitespace-nowrap text-xs tabular-nums sm:table-cell"><Time value={notification.due_at} />{#if notification.awaiting_confirmation}<p class="text-base-content/60">Finale unconfirmed</p>{/if}</td>
                                                <td><span class={`badge badge-sm ${notification.awaiting_confirmation ? 'badge-warning badge-soft' : stateClasses[notification.state]}`}>{notification.awaiting_confirmation ? 'Awaiting finale' : stateLabels[notification.state]}</span></td>
                                                <td class="text-right"><button class="btn btn-ghost btn-sm btn-square" aria-label={`Preview ${title} notification`} onclick={() => { selectedNotification = notification; previewDialog?.showModal(); }}><svg class="size-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><path d="m9 5 7 7-7 7" /></svg></button></td>
                                            </tr>
                                        {/each}
                                    </tbody>
                                </table>
                            </div>
                        {:else}
                            <div class="card-body items-center py-10 text-center">
                                <h3 class="card-title">{offset ? 'No more notifications' : activityView === 'upcoming' ? 'Nothing scheduled yet' : 'No deliveries yet'}</h3>
                                <p class="max-w-md text-base-content/70">{activityView === 'upcoming' ? 'Eligible missing episodes appear after a Sonarr scan. Episodes from before tracking began are skipped.' : 'Completed delivery attempts will appear here. Each notification is sent at most once.'}</p>
                                <a class="btn btn-sm" href="/shows">Manage shows</a>
                            </div>
                        {/if}
                    </div>
                    <div class="flex flex-wrap items-center justify-between gap-3">
                        <span class="text-sm text-base-content/70">{notifications.length ? `Showing ${offset + 1}–${offset + notifications.length}` : 'No entries'}</span>
                        <nav class="join" aria-label="Activity pages">
                            <button class="btn btn-sm join-item" aria-label="Previous notifications" disabled={locked || offset === 0} onclick={() => refresh(Math.max(0, offset - pageSize))}>«</button>
                            <span class="btn btn-sm join-item btn-active" aria-current="page">Page {Math.floor(offset / pageSize) + 1}</span>
                            <button class="btn btn-sm join-item" aria-label="Next notifications" disabled={locked || !hasNext} onclick={() => refresh(offset + pageSize)}>»</button>
                        </nav>
                    </div>
                </section>
                <dialog onclose={() => selectedNotification = null} bind:this={previewDialog} class="modal modal-bottom sm:modal-middle" aria-labelledby="notification-preview-title">
                    <div class="modal-box space-y-4">
                        {#if selectedNotification}
                            {@const notification = selectedNotification}
                            <div class="flex items-start justify-between gap-4">
                                <div><p class="text-xs uppercase tracking-wider text-base-content/60">Discord notification</p><h2 id="notification-preview-title" class="text-lg font-semibold">{shows.find((show) => show.id === notification.series_id)?.title ?? `Show ${notification.series_id}`}</h2><p class="text-sm text-base-content/60">Season {notification.season} · {notification.mode === 'season' ? 'Full season' : 'Episode'}</p></div>
                                <form method="dialog"><button class="btn btn-ghost btn-sm btn-square" aria-label="Close preview">✕</button></form>
                            </div>
                            <dl class="grid grid-cols-1 gap-3 text-sm sm:grid-cols-2">
                                <div><dt class="text-base-content/60">{notification.awaiting_confirmation ? 'Latest listed air time · finale unconfirmed' : 'Air time'}</dt><dd><Time value={notification.due_at} /></dd></div>
                                <div><dt class="text-base-content/60">Delivery status</dt><dd>{notification.awaiting_confirmation ? 'Awaiting finale confirmation' : stateLabels[notification.state]}</dd></div>
                                {#if notification.sent_at}<div><dt class="text-base-content/60">Sent</dt><dd><Time value={notification.sent_at} /></dd></div>{:else if notification.attempted_at}<div><dt class="text-base-content/60">Last attempt</dt><dd><Time value={notification.attempted_at} /></dd></div>{/if}
                            </dl>
                            {#if notification.state === 'uncertain' || notification.state === 'sending'}<div class="alert alert-warning alert-soft text-sm">Delivery could not be confirmed. This attempt will not be repeated to prevent duplicates.</div>{/if}
                            {#if notification.state === 'failed'}<div class="alert alert-error alert-soft text-sm">Discord rejected this notification. Check Server Core’s logs for details.</div>{/if}
                            {#if notification.state === 'covered'}<p class="text-sm text-base-content/60">These episodes were already covered by an earlier notification.</p>{/if}
                            <EmbedPreview color={colorHex(notification.mode === 'season' ? colors.season_color : colors.episode_color)}>
                                <p class="whitespace-pre-wrap">{notification.content}</p><ShowPoster id={notification.series_id} />
                            </EmbedPreview>
                            <p class="text-xs text-base-content/60">Artwork is included when available. Tests send to your configured channel without changing scheduled delivery.</p>
                            <div class="modal-action"><button class="btn" disabled={locked} onclick={() => sendTest(notification)}>{#if busy === `test:${notification.key}`}<span class="loading loading-spinner loading-xs" aria-hidden="true"></span>{/if}{busy === `test:${notification.key}` ? 'Sending test…' : 'Send test to Discord'}</button></div>
                        {/if}
                    </div>
                    <form method="dialog" class="modal-backdrop"><button>Close preview</button></form>
                    <div class="toast toast-end toast-bottom z-50 max-w-full" role="status" aria-live="polite" aria-atomic="true">
                        {#if toast}
                            <div class="alert max-w-sm" class:alert-success={toast.ok} class:alert-warning={!toast.ok}>
                                <span>{toast.message}</span>
                                <button class="btn btn-ghost btn-sm" onclick={dismissToast} aria-label="Dismiss notification">✕</button>
                            </div>
                        {/if}
                    </div>
                </dialog>
            {/if}
		</div>
	{:else}<div class="grid justify-items-center gap-3 py-10 text-center"><h2 class="card-title">Waiting for Server Core</h2><p>Start the backend on port 8090, then retry the connection.</p></div>{/if}
	<footer class="footer sm:footer-horizontal justify-between gap-4 py-6 text-base-content/70"><span>Jelly Alert · Sonarr → Discord</span><span>Refreshes every 30 seconds while visible · Updated <Time value={updatedAt} empty="—" /></span></footer>
</div>
