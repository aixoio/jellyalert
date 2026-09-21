<script lang="ts">
	import { onMount } from 'svelte';
	import { api, errorMessage, colorHex, type EmbedColors, type Health, type Notification, type Show, type DeliveryState } from '$lib/api';
	import Time from '$lib/Time.svelte';
	import ShowPoster from '$lib/ShowPoster.svelte';

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
    <header class="section-heading page-heading">
        <div class="heading-copy"><h1 class="card-title">{titles[section]}</h1><p>{descriptions[section]}</p></div>
		<button class="btn btn-outline" onclick={() => refresh()} disabled={locked}>
			{#if refreshing}<span class="loading loading-spinner loading-xs" aria-hidden="true"></span>{/if}
			{refreshing ? 'Refreshing' : 'Refresh'}
		</button>
	</header>

	{#if error}
		<div role="alert" class="alert alert-error notice">
			<div><strong>{updatedAt ? 'Unable to update' : 'Unable to connect'}</strong><p>{error}</p>{#if updatedAt}<p>Showing the last successful refresh. <Time value={updatedAt} /></p>{/if}</div>
			<button class="btn btn-sm" onclick={() => refresh()} disabled={locked}>Retry</button>
		</div>
	{/if}
	<div class="toast toast-end toast-bottom z-50 max-w-full" role="status" aria-live="polite" aria-atomic="true">
		{#if toast}
			<div class="alert max-w-sm" class:alert-success={toast.ok} class:alert-warning={!toast.ok}>
				<span>{toast.message}</span>
				<button class="btn btn-ghost btn-sm" onclick={dismissToast} aria-label="Dismiss notification">✕</button>
			</div>
		{/if}
	</div>

	{#if loading}
		<div class="empty-state" role="status"><span class="loading loading-spinner loading-lg"></span><p>Connecting to Server Core…</p></div>
	{:else if updatedAt}
		<div class="page-stack">
			{#if section === 'overview' && health}
			<div class="dashboard">
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
						<div class="section-heading"><h2 id="status-heading" class="card-title">Service status</h2><span class="badge" class:badge-success={!error} class:badge-warning={!!error}>{error ? 'Stale' : 'Connected'}</span></div>
						<dl class="status-list">
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
			<section id="shows" class="page-stack" aria-label="Show preferences">
				<div class="page-stack">
					<div class="section-heading"><p>Changes save automatically. Manage the shared default in Settings.</p><div class="actions"><span class="badge badge-primary badge-soft">{tracked} tracked</span><span class="badge badge-ghost">{excluded} excluded</span></div></div>
                    <div class="card card-border"><div class="card-body show-toolbar">
                        <label class="fieldset search-field"><span class="fieldset-legend">Search shows</span><input class="input" type="search" placeholder="Search by title…" bind:value={query} /></label>
                        <label class="fieldset filter-field"><span class="fieldset-legend">Show status</span><select class="select" aria-label="Filter shows" bind:value={filter}><option value="active">In Sonarr</option><option value="tracked">Tracked</option><option value="excluded">Excluded</option><option value="removed">Removed from Sonarr</option><option value="all">All shows</option></select></label>
                        <button class="btn btn-ghost" disabled={!query && filter === 'active'} onclick={() => { query = ''; filter = 'active'; }}>Clear filters</button>
                    </div></div>
					{#if visibleShows.length}
						<div class="shows-grid">
                            {#each pagedShows as show (show.id)}
                                <article class="card card-border show-card" aria-label={show.title}>
                                    <div class="card-body">
                                        <div class="section-heading"><span class="badge" class:badge-ghost={!show.active || show.excluded} class:badge-success={show.active && !show.excluded} class:badge-soft={show.active && !show.excluded}>{!show.active ? 'Removed from Sonarr' : show.excluded ? 'Excluded' : 'Tracked'}</span>{#if busy === `show:${show.id}` || busy === `mode:${show.id}`}<span class="loading loading-spinner loading-xs" aria-label="Saving"></span>{/if}</div>
                                        <a class="show-identity link link-hover" href={`/shows/${show.id}`} aria-label={`View ${show.title} progress`}><ShowPoster id={show.id} /><h2 class="card-title show-name">{show.title}</h2></a>
                                        <div class="show-controls">
                                            <label class="fieldset"><span class="fieldset-legend">Notify me</span><select class="select" aria-label={`Notification mode for ${show.title}`} value={show.mode_overridden ? show.mode : 'default'} disabled={locked || !show.active}
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
                                            <label class="tracking-control"><span>Track this show</span><input type="checkbox" class="toggle toggle-primary" aria-label={`Track ${show.title}`} checked={!show.excluded} disabled={locked || !show.active} onchange={(event) => { event.currentTarget.checked = !show.excluded; void change(`show:${show.id}`, `shows/${show.id}/exclusion`, 'PUT', { excluded: !show.excluded }, `${show.title} ${show.excluded ? 'is now tracked' : 'is now excluded'}.`); }} /></label>
                                            <p>{!show.active ? 'Restore this show in Sonarr to resume tracking.' : show.excluded ? 'Notifications are off. Your preference is kept.' : show.mode === 'season' ? 'One alert when a full season has aired.' : 'An alert when each missing episode airs.'}</p>
                                        </div>
                                    </div>
                                </article>
                            {/each}
                        </div>
					{:else}<div class="empty-state"><h3 class="card-title">{shows.length ? 'No matching shows' : 'Your shows will appear here'}</h3><p>{shows.length ? 'Try another search or filter.' : 'Add and monitor shows in Sonarr. Jelly Alert imports them on its next scan.'}</p>{#if query || filter !== 'active'}<button class="btn btn-ghost btn-sm" onclick={() => { query = ''; filter = 'active'; }}>Clear filters</button>{/if}</div>{/if}
					<div class="section-heading"><span>{visibleShows.length ? `Showing ${showPage * 12 + 1}–${Math.min((showPage + 1) * 12, visibleShows.length)} of ${visibleShows.length}` : 'No shows'}</span><div class="join"><button class="btn btn-sm join-item" disabled={showPage === 0} onclick={() => showPage--} aria-label="Previous shows">Previous</button><button class="btn btn-sm join-item" disabled={(showPage + 1) * 12 >= visibleShows.length} onclick={() => showPage++} aria-label="Next shows">Next</button></div></div>
					<p>Sonarr’s monitoring and library status also determine which episodes are eligible. Existing notification history is kept when you exclude a show.</p>
				</div>
			</section>

            {/if}
            {#if section === 'activity'}
			<section id="activity" class="page-stack">
				<div class="page-stack">
					<div class="section-heading"><div><h2 class="card-title">Notification activity</h2><p>{activityView === 'upcoming' ? 'Scheduled alerts and seasons awaiting finale confirmation, with the next listed air time first.' : 'Past delivery attempts, with the latest air time first.'}</p></div><span class="badge badge-outline">Your local time</span></div>
					<div class="tabs tabs-box" aria-label="Activity view"><button class="tab" class:tab-active={activityView === 'upcoming'} aria-pressed={activityView === 'upcoming'} disabled={locked} onclick={() => refresh(0, 'upcoming')}>Upcoming</button><button class="tab" class:tab-active={activityView === 'history'} aria-pressed={activityView === 'history'} disabled={locked} onclick={() => refresh(0, 'history')}>Delivery history</button></div>
					{#if notifications.length}<ol class="activity-list">
                            {#each notifications as notification (notification.key)}
                                <li class="card card-border">
                                    <div class="card-body">
                                        <div class="activity-entry">
                                            <div class="activity-copy">
                                                <span class="badge badge-outline">{notification.mode === 'season' ? 'Full season' : 'Episode'} · Season {notification.season}</span>
                                                <h2 class="card-title"><a class="link link-hover" href={`/shows/${notification.series_id}`} aria-label={`View ${shows.find((show) => show.id === notification.series_id)?.title ?? `show ${notification.series_id}`} progress`}>{shows.find((show) => show.id === notification.series_id)?.title ?? `Show ${notification.series_id}`}</a></h2>
                                                <details class="message-preview">
                                                    <summary>{notification.awaiting_confirmation ? 'Preview waiting notice' : 'Preview Discord embed'}</summary>
                                                    <div class="discord-preview" style:border-left-color={colorHex(notification.mode === 'season' ? colors.season_color : colors.episode_color)}>
                                                        <strong>Jelly Name</strong>
                                                        <p>{notification.content}</p>
                                                        <ShowPoster id={notification.series_id} />
                                                    </div>
                                                    <p class="test-help">Series artwork is included when available. If artwork cannot load, the alert still sends with its text.</p>
                                                </details>
                                            </div>
                                            <dl class="activity-meta">
                                                <div><dt>{notification.awaiting_confirmation ? 'Latest listed air time (not confirmed finale)' : 'Air time'}</dt><dd><Time value={notification.due_at} /></dd></div>
                                                <div><dt>Delivery status</dt><dd><span class={`badge ${stateClasses[notification.state]}`}>{notification.awaiting_confirmation ? 'Awaiting finale confirmation' : stateLabels[notification.state]}</span></dd></div>
                                                {#if notification.sent_at}<div><dt>Sent at</dt><dd><Time value={notification.sent_at} /></dd></div>{:else if notification.attempted_at}<div><dt>Attempted at</dt><dd><Time value={notification.attempted_at} /></dd></div>{/if}
                                            </dl>
                                        </div>
                                        <div class="card-actions"><button class="btn btn-outline btn-sm" disabled={locked} onclick={() => sendTest(notification)}>{busy === `test:${notification.key}` ? 'Sending test…' : 'Send test to Discord'}</button></div>
                                        <p class="test-help">Sends the preview to your configured Discord channel. Testing does not mark this notification as sent or remove it from normal delivery.</p>
                                        {#if notification.state === 'uncertain' || notification.state === 'sending'}<div class="alert alert-warning"><p>Delivery could not be confirmed. This attempt will not be repeated to prevent duplicate notifications.</p></div>{/if}
                                        {#if notification.state === 'failed'}<div class="alert alert-error"><p>Discord rejected this notification. Check Server Core’s logs for details.</p></div>{/if}
                                        {#if notification.state === 'covered'}<p>These episodes were already covered by an earlier notification.</p>{/if}
                                    </div>
                                </li>
                            {/each}
                        </ol>{:else}<div class="empty-state"><h3 class="card-title">{offset ? 'No more notifications' : activityView === 'upcoming' ? 'Nothing scheduled yet' : 'No delivery history yet'}</h3><p>{activityView === 'upcoming' ? 'Eligible missing episodes appear after a Sonarr scan. Episodes from before tracking began are skipped.' : 'Completed delivery attempts will appear here. Each notification is sent at most once.'}</p></div>{/if}
					<div class="section-heading"><span>{notifications.length ? `Showing ${offset + 1}–${offset + notifications.length}` : 'No entries'}</span><div class="join"><button class="btn btn-sm join-item" aria-label="Previous notifications" disabled={locked || offset === 0} onclick={() => refresh(Math.max(0, offset - pageSize))}>Previous</button><button class="btn btn-sm join-item" aria-label="Next notifications" disabled={locked || !hasNext} onclick={() => refresh(offset + pageSize)}>Next</button></div></div>
				</div>
			</section>
            {/if}
		</div>
	{:else}<div class="empty-state"><h2 class="card-title">Waiting for Server Core</h2><p>Start the backend on port 8090, then retry the connection.</p></div>{/if}
	<footer class="page-footer"><span>Jelly Alert · Sonarr → Discord</span><span>Refreshes every 30 seconds while visible · Updated <Time value={updatedAt} empty="—" /></span></footer>
</div>

<style>
    .message-preview summary { cursor: pointer; width: fit-content; font-weight: 600; padding: 0.5rem 0; }
    .message-preview summary:focus-visible { outline: 2px solid #5865f2; outline-offset: 4px; }
    .discord-preview { border-left: 4px solid #5865f2; max-width: 32rem; margin-top: 0.5rem; padding: 1rem; border-radius: 0.5rem; background: #313338; color: #dbdee1; overflow-wrap: anywhere; }
    .discord-preview strong { color: #f2f3f5; }
    .discord-preview p { margin-top: 0.4rem; white-space: pre-wrap; }
    .discord-preview :global(.show-poster) { margin: 0.75rem 0 0; width: 120px; }
    .discord-preview :global(img) { width: 120px; height: auto; border-radius: 4px; }
    .test-help { font-size: 0.875rem; }
</style>
