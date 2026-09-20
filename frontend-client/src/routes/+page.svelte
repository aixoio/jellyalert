<script lang="ts">
	import { onMount } from 'svelte';
	import { api, errorMessage, type Health, type Notification, type Show, type DeliveryState } from '$lib/api';
	import Time from '$lib/Time.svelte';

	let health = $state<Health | null>(null);
	let shows = $state<Show[]>([]);
	let notifications = $state<Notification[]>([]);
	let loading = $state(true);
	let refreshing = $state(false);
	let busy = $state('');
	let error = $state('');
	let feedback = $state('');
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
			const [nextHealth, nextShows, nextNotifications] = await Promise.all([
				api<Health>('health'), api<Show[]>('shows'),
				api<Notification[]>(`notifications?limit=${pageSize + 1}&offset=${pageOffset}&view=${view}`)
			]);
			if (!alive) return;
			health = nextHealth;
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
		feedback = '';
		error = '';
		try {
			await api<void>(path, { method, body: body === undefined ? undefined : JSON.stringify(body) });
			if (name.startsWith('show:')) {
				const id = Number(name.slice(5));
				shows = shows.map((show) => show.id === id ? { ...show, excluded: !show.excluded } : show);
			}
			saved?.();
			feedback = success;
			busy = '';
			await refresh(0);
		} catch (cause) {
			error = `${errorMessage(cause)} Refresh to confirm the current setting before retrying.`;
		} finally { busy = ''; }
	}

	onMount(() => {
		alive = true;
		void refresh();
		const timer = setInterval(() => { if (!document.hidden) void refresh(); }, 30_000);
		const visible = () => { if (!document.hidden) void refresh(); };
		document.addEventListener('visibilitychange', visible);
		return () => { alive = false; clearInterval(timer); document.removeEventListener('visibilitychange', visible); };
	});
</script>

<svelte:head>
	<title>Jelly Alert · Notification control</title>
	<meta name="description" content="Control episode and season alerts, choose which shows to track, and follow Discord deliveries." />
</svelte:head>

<div class="shell">
	<header class="app-header">
		<div class="brand">
			<svg class="brand-mark" viewBox="0 0 40 40" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M8 23v-5a12 12 0 0 1 24 0v5Z"/><path d="M12 23v7q0 5 4 5m4-12v9m8-9v7q0 5-4 5" stroke-linecap="round"/><path d="M15 17h.01M25 17h.01" stroke-width="3" stroke-linecap="round"/></svg>
			<div><h1 class="card-title">Jelly Alert</h1><p>Notification control</p></div>
		</div>
		<nav aria-label="Main navigation" class="join">
			<a class="btn btn-ghost join-item" href="#status">Status</a>
			<a class="btn btn-ghost join-item" href="#shows">Shows</a>
			<a class="btn btn-ghost join-item" href="#activity">Activity</a>
		</nav>
		<button class="btn btn-outline btn-sm" onclick={() => refresh()} disabled={locked}>
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
	{#if feedback}
		<div role="status" class="alert alert-success notice"><span>{feedback}</span><button class="btn btn-ghost btn-sm" onclick={() => feedback = ''} aria-label="Dismiss confirmation">Dismiss</button></div>
	{/if}

	{#if loading}
		<div class="empty-state" role="status"><span class="loading loading-spinner loading-lg"></span><p>Connecting to Server Core…</p></div>
	{:else if health}
		<main class="page-stack">
			<div class="dashboard">
                <section class="card card-border">
                    <div class="card-body">
                        <h2 class="card-title">Your shows, your schedule</h2>
                        <p>Choose every episode or full seasons for each show below. Changes save immediately and only affect that show.</p>
                        <p><strong>Every episode:</strong> an alert at each missing episode’s air time.</p>
                        <p><strong>Full seasons:</strong> one alert when the complete season has aired and episodes are missing. Sonarr must confirm season completion.</p>
                        <p>New shows start with episode alerts. Turn tracking off to exclude a show entirely.</p>
                        <p>Air times come from Sonarr and do not confirm that a download is available.</p>
                        <div class="card-actions"><a class="btn btn-primary" href="#shows">Manage show notifications</a></div>
                    </div>
                </section>

				<aside id="status" class="card card-border" aria-labelledby="status-heading">
					<div class="card-body">
						<div class="section-heading"><h2 id="status-heading" class="card-title">Service status</h2><span class="badge" class:badge-success={!error} class:badge-warning={!!error}>{error ? 'Stale' : 'Connected'}</span></div>
						<dl class="status-list">
							<div><dt>Sonarr scan</dt><dd><span class="badge" class:badge-success={health.worker.last_scan_succeeded} class:badge-warning={!health.worker.last_scan_succeeded}>{health.worker.last_scan_at === null ? 'Waiting for first scan' : health.worker.last_scan_succeeded ? 'Healthy' : 'Retrying'}</span></dd></div>
							<div><dt>Last scan</dt><dd><Time value={health.worker.last_scan_at} /></dd></div>
							<div><dt>Discord delivery</dt><dd><span class="badge" class:badge-error={health.webhook_disabled} class:badge-warning={!health.webhook_disabled && health.webhook_retry_at > (updatedAt ?? 0)} class:badge-success={!health.webhook_disabled && health.webhook_retry_at <= (updatedAt ?? 0)}>{health.webhook_disabled ? 'Paused' : health.webhook_retry_at > (updatedAt ?? 0) ? 'Waiting to retry' : 'Ready'}</span></dd></div>
							<div><dt>Last delivery attempt</dt><dd><Time value={health.worker.last_delivery_at} /></dd></div>
							<div><dt>Tracking since</dt><dd><Time value={health.tracking_since} /></dd></div>
						</dl>
						{#if health.webhook_disabled}<div class="alert alert-warning"><p>Discord rejected the webhook. Update the webhook in Server Core’s configuration and restart it, then resume notifications here.</p></div><button class="btn btn-outline" disabled={locked} onclick={() => change('resume', 'webhook/resume', 'POST', undefined, 'Discord notifications resumed.')}>{busy === 'resume' ? 'Resuming…' : 'Resume notifications'}</button>{/if}
						{#if !health.webhook_disabled && health.webhook_retry_at > (updatedAt ?? 0)}<p>Automatic retry after <Time value={health.webhook_retry_at} />.</p>{/if}
						{#if health.unresolved_deliveries > 0}<div class="alert alert-warning"><span>{health.unresolved_deliveries} delivery attempt{health.unresolved_deliveries === 1 ? ' needs' : 's need'} attention. See activity for details. Unconfirmed attempts are not retried to avoid duplicates.</span></div>{/if}
					</div>
				</aside>
			</div>

			<section id="shows" class="card card-border">
				<div class="card-body">
					<div class="section-heading"><div><h2 class="card-title">Your shows</h2><p>Turn tracking off to stop all notifications for a show.</p></div><div class="actions"><span class="badge badge-primary badge-soft">{tracked} tracked</span><span class="badge badge-ghost">{excluded} excluded</span></div></div>
					<div class="toolbar">
						<input class="input" type="search" aria-label="Search shows" placeholder="Search your shows…" bind:value={query} />
						<select class="select" aria-label="Filter shows" bind:value={filter}><option value="active">In Sonarr</option><option value="tracked">Tracked</option><option value="excluded">Excluded</option><option value="removed">Removed from Sonarr</option><option value="all">All shows</option></select>
						<span>{visibleShows.length} show{visibleShows.length === 1 ? '' : 's'}</span>
					</div>
					{#if visibleShows.length}
						<div class="scroll-region"><table class="table"><thead><tr><th scope="col">Show</th><th scope="col">Track</th></tr></thead><tbody>
							{#each pagedShows as show (show.id)}<tr><td class="show-name"><strong>{show.title}</strong><p><span class="badge" class:badge-ghost={!show.active || show.excluded} class:badge-success={show.active && !show.excluded} class:badge-soft={show.active && !show.excluded}>{!show.active ? 'Removed' : show.excluded ? 'Excluded' : 'Tracked'}</span></p>
                                <label class="fieldset">
                                    <span class="fieldset-legend">Notify me</span>
                                    <select class="select select-sm" aria-label={`Notification mode for ${show.title}`} value={show.mode} disabled={locked || !show.active}
                                        onchange={(event) => {
                                            const mode = event.currentTarget.value;
                                            event.currentTarget.value = show.mode;
                                            if (mode !== 'episode' && mode !== 'season') return;
                                            void change(`mode:${show.id}`, `shows/${show.id}/mode`, 'PUT', { mode }, `${show.title}: ${mode === 'episode' ? 'every episode' : 'full seasons'} saved.`, () => {
                                                shows = shows.map((item) => item.id === show.id ? { ...item, mode } : item);
                                            });
                                        }}>
                                        <option value="episode">Every episode</option>
                                        <option value="season">Full seasons</option>
                                    </select>
                                </label>
                            </td><td><input type="checkbox" class="toggle toggle-primary" aria-label={`Track ${show.title}`} checked={!show.excluded} disabled={locked || !show.active} onchange={(event) => { event.currentTarget.checked = !show.excluded; void change(`show:${show.id}`, `shows/${show.id}/exclusion`, 'PUT', { excluded: !show.excluded }, `${show.title} ${show.excluded ? 'is now tracked' : 'is now excluded'}.`); }} /></td></tr>{/each}
						</tbody></table></div>
					{:else}<div class="empty-state"><h3 class="card-title">{shows.length ? 'No matching shows' : 'Your shows will appear here'}</h3><p>{shows.length ? 'Try another search or filter.' : 'Add and monitor shows in Sonarr. Jelly Alert imports them on its next scan.'}</p>{#if query || filter !== 'active'}<button class="btn btn-ghost btn-sm" onclick={() => { query = ''; filter = 'active'; }}>Clear filters</button>{/if}</div>{/if}
					<div class="section-heading"><span>{visibleShows.length ? `Showing ${showPage * 12 + 1}–${Math.min((showPage + 1) * 12, visibleShows.length)} of ${visibleShows.length}` : 'No shows'}</span><div class="join"><button class="btn btn-sm join-item" disabled={showPage === 0} onclick={() => showPage--} aria-label="Previous shows">Previous</button><button class="btn btn-sm join-item" disabled={(showPage + 1) * 12 >= visibleShows.length} onclick={() => showPage++} aria-label="Next shows">Next</button></div></div>
					<p>Sonarr’s monitoring and library status also determine which episodes are eligible. Existing notification history is kept when you exclude a show.</p>
				</div>
			</section>

			<section id="activity" class="card card-border">
				<div class="card-body">
					<div class="section-heading"><div><h2 class="card-title">Notification activity</h2><p>{activityView === 'upcoming' ? 'Eligible alerts for each show’s preference, with the next air time first.' : 'Past delivery attempts, with the latest air time first.'}</p></div><span class="badge badge-outline">Your local time</span></div>
					<div class="join" aria-label="Activity view"><button class="btn btn-sm join-item" class:btn-active={activityView === 'upcoming'} aria-pressed={activityView === 'upcoming'} disabled={locked} onclick={() => refresh(0, 'upcoming')}>Upcoming</button><button class="btn btn-sm join-item" class:btn-active={activityView === 'history'} aria-pressed={activityView === 'history'} disabled={locked} onclick={() => refresh(0, 'history')}>Delivery history</button></div>
					{#if notifications.length}<div class="scroll-region"><table class="table"><thead><tr><th scope="col">Notification</th><th scope="col">Air time</th><th scope="col">Delivery</th></tr></thead><tbody>
						{#each notifications as notification (notification.key)}<tr><td class="delivery-copy"><strong>{shows.find((show) => show.id === notification.series_id)?.title ?? `Show ${notification.series_id}`}</strong><p>{notification.mode === 'season' ? 'Full season' : 'Episode'} · Season {notification.season}</p><details class="collapse collapse-arrow"><summary class="collapse-title">Message details</summary><div class="collapse-content"><p>{notification.content}</p>{#if notification.state === 'uncertain' || notification.state === 'sending'}<p>Delivery could not be confirmed. This attempt will not be repeated to prevent duplicate notifications.</p>{/if}{#if notification.state === 'failed'}<p>Discord rejected this notification. Check Server Core’s logs for details.</p>{/if}{#if notification.state === 'covered'}<p>These episodes were already covered by an earlier notification.</p>{/if}</div></details></td><td><Time value={notification.due_at} /></td><td><span class={`badge ${stateClasses[notification.state]}`}>{stateLabels[notification.state]}</span>{#if notification.sent_at}<p><Time value={notification.sent_at} /></p>{:else if notification.attempted_at}<p><Time value={notification.attempted_at} /></p>{/if}</td></tr>{/each}
					</tbody></table></div>{:else}<div class="empty-state"><h3 class="card-title">{offset ? 'No more notifications' : activityView === 'upcoming' ? 'Nothing scheduled yet' : 'No delivery history yet'}</h3><p>{activityView === 'upcoming' ? 'Eligible missing episodes appear after a Sonarr scan. Episodes from before tracking began are skipped.' : 'Completed delivery attempts will appear here. Each notification is sent at most once.'}</p></div>{/if}
					<div class="section-heading"><span>{notifications.length ? `Showing ${offset + 1}–${offset + notifications.length}` : 'No entries'}</span><div class="join"><button class="btn btn-sm join-item" aria-label="Previous notifications" disabled={locked || offset === 0} onclick={() => refresh(Math.max(0, offset - pageSize))}>Previous</button><button class="btn btn-sm join-item" aria-label="Next notifications" disabled={locked || !hasNext} onclick={() => refresh(offset + pageSize)}>Next</button></div></div>
				</div>
			</section>
		</main>
	{:else}<main class="empty-state"><h2 class="card-title">Waiting for Server Core</h2><p>Start the backend on port 8090, then retry the connection.</p></main>{/if}
	<footer class="page-footer"><span>Jelly Alert · Sonarr → Discord</span><span>Refreshes every 30 seconds while visible · Updated <Time value={updatedAt} empty="—" /></span></footer>
</div>
