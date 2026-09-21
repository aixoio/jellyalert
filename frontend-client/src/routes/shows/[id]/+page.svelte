<script lang="ts">
	import { page } from '$app/state';
	import { api, errorMessage, type SeriesProgress, type ProgressCounts } from '$lib/api';
	import Time from '$lib/Time.svelte';
	import ShowPoster from '$lib/ShowPoster.svelte';

	let detail = $state<SeriesProgress | null>(null);
	let loading = $state(true);
	let error = $state('');
	let now = $state(Math.floor(Date.now() / 1000));
	let retry = $state(0);
	const percent = (value: number, total: number) => total ? Math.round(value / total * 100) : 0;
	function remaining(at: number) {
		const seconds = Math.max(0, at - now);
		if (!seconds) return 'Air time reached';
		const minutes = Math.ceil(seconds / 60);
		const days = Math.floor(minutes / 1440);
		const hours = Math.floor(minutes % 1440 / 60);
		return days ? `${days}d ${hours}h remaining` : hours ? `${hours}h ${minutes % 60}m remaining` : `${minutes}m remaining`;
	}
	$effect(() => {
		const id = page.params.id;
		retry;
		const controller = new AbortController();
		let active = true;
		let fetching = false;
		detail = null;
		async function load() {
			if (fetching) return;
			fetching = true;
			try {
				const result = await api<SeriesProgress>(`shows/${id}`, { signal: AbortSignal.any([controller.signal, AbortSignal.timeout(35_000)]) });
				if (active) { detail = result; error = ''; }
			} catch (cause) { if (active) error = errorMessage(cause); }
			finally { fetching = false; if (active) loading = false; }
		}
		loading = true;
		void load();
		const timer = setInterval(() => { now = Math.floor(Date.now() / 1000); }, 1000);
		const refresh = setInterval(() => { if (!document.hidden) void load(); }, 30_000);
		return () => { active = false; controller.abort(); clearInterval(timer); clearInterval(refresh); };
	});
</script>

<svelte:head><title>{detail?.show.title ?? 'Series'} · Jelly Alert</title></svelte:head>

{#snippet progress(counts: ProgressCounts, label: string)}
	<div class="progress-pair">
		<div><div class="section-heading"><span>Aired</span><strong>{counts.aired} / {counts.total}</strong></div><progress class="progress progress-primary" value={counts.aired} max={counts.total || 1} aria-label={`${label}: episodes aired`}></progress></div>
		<div><div class="section-heading"><span>In your library</span><strong>{counts.in_library} / {counts.total}</strong></div><progress class="progress progress-secondary" value={counts.in_library} max={counts.total || 1} aria-label={`${label}: episodes in library`}></progress></div>
	</div>
{/snippet}

<div class="page-stack">
	<a class="link" href="/shows">← All shows</a>
	{#if error}<div class="alert alert-error" role="alert"><span>{error}{detail ? ' Showing the last successful update.' : ''}</span><button class="btn btn-sm" onclick={() => retry++}>Retry</button></div>{/if}
	{#if loading}<p role="status">Loading series progress…</p>
	{:else if detail}
		<header class="section-heading">
			<div class="show-identity"><ShowPoster id={detail.show.id} /><div class="heading-copy"><h1 class="card-title">{detail.show.title}</h1><div class="actions"><span class="badge badge-outline">{detail.status}</span><span class="badge badge-primary badge-soft">{detail.show.mode === 'season' ? 'Full-season alerts' : 'Episode alerts'}</span></div><p>{detail.show.mode_overridden ? 'Individual notification preference' : 'Following your default preference'}</p></div></div>
		</header>
		{#if detail.notification_block}<div class="alert alert-warning">{detail.notification_block}</div>{/if}
		<section class="card card-border"><div class="card-body">
			<div class="section-heading"><h2 class="card-title">Series progress</h2><span class="badge badge-outline">{percent(detail.counts.aired, detail.counts.total)}% of listed episodes aired</span></div>
			{@render progress(detail.counts, 'Series')}
			<p>{detail.status === 'ended' ? 'Series marked ended in Sonarr.' : 'The series is ongoing or its end is unconfirmed. More episodes may be added.'} Progress covers known regular episodes; specials are listed separately.</p>
			{#if detail.counts.undated}<p>{detail.counts.undated} episode(s) have no confirmed air date.</p>{/if}
			{#if !detail.counts.total}<p>No regular episodes are listed in Sonarr yet.</p>{/if}
		</div></section>
		<div class="dashboard">
			<section class="card card-border"><div class="card-body">
				<h2 class="card-title">Next release</h2>
				{#if detail.next_release}
					{@const release = detail.next_release}
					<p class="countdown-text">{remaining(release.air_at)}</p>
					<p>Season {release.season} · Episode {release.episode}: {release.title}</p>
					<Time value={release.air_at} />
					<p>Scheduled air time from Sonarr. Download availability may differ.</p>
				{:else}<p>No future air date is listed. New dates will appear when Sonarr updates.</p>{/if}
			</div></section>
			<section class="card card-border"><div class="card-body">
				<h2 class="card-title">Next notification</h2>
				{#if detail.next_notification}
					{@const notification = detail.next_notification}
					<p class="countdown-text">{notification.episodes_remaining} episode{notification.episodes_remaining === 1 ? '' : 's'} still to air</p>
					<p>Season {notification.season}{notification.episode !== null ? ` · Episode ${notification.episode}` : ''}</p>
					{#if notification.awaiting_confirmation}
						<span class="badge badge-warning">Awaiting finale confirmation</span>
						<p>This is the remaining count among listed episodes. The final season total and notification date are unconfirmed.</p>
					{:else}
						<p>{remaining(notification.due_at)} · <Time value={notification.due_at} /></p>
						<p>{detail.notification_block ? 'Delivery is blocked for the reason shown above.' : 'Expected notification time. Sonarr is checked again before sending; downloads, schedule changes, or delivery retries can change this.'}</p>
					{/if}
				{:else}<p>No eligible notification is currently planned.</p><p>Alerts require monitored, missing episodes within the tracking window that have not already been notified. Season alerts also need complete episode dates and numbering.</p>{/if}
				<a class="link" href="/activity">View notification activity</a>
			</div></section>
		</div>
		<section class="page-stack" aria-label="Season and episode progress">
			<h2 class="card-title">Seasons & episodes</h2>
			{#each detail.seasons.toReversed() as season (season.number)}
				<details class="card card-border season-card" open={season.number === Math.max(...detail.seasons.map((item) => item.number))}>
					<summary class="season-summary"><strong>{season.number ? `Season ${season.number}` : 'Specials'}</strong><span>{season.counts.aired} / {season.counts.total} aired · {season.counts.in_library} in library</span></summary>
					<div class="card-body">
						{@render progress(season.counts, `Season ${season.number}`)}
						{#if season.completion_confirmed}<p>{season.final_air_at !== null && season.final_air_at <= now ? 'All listed episodes have aired.' : 'Confirmed final scheduled air time:'} <Time value={season.final_air_at} /></p>
						{:else if season.number > 0}<p>Season completion is unconfirmed.{season.counts.undated ? ` ${season.counts.undated} episode(s) still need air dates.` : ' Sonarr may still add or update episodes.'}</p>
						{:else}<p>Specials have no season completion boundary. They can receive individual episode alerts in episode mode.</p>{/if}
						{#if season.notification}<p>{season.notification.episodes_remaining} listed episode(s) still to air before {season.notification.episode !== null ? `episode ${season.notification.episode}’s alert` : 'the season alert'}{season.notification.awaiting_confirmation ? '; finale confirmation also required.' : '.'}</p>{/if}
						<div class="scroll-region"><table class="table">
							<caption class="sr-only">Episodes in {season.number ? `season ${season.number}` : 'specials'}</caption>
							<thead><tr><th scope="col">Episode</th><th scope="col">Air time</th><th scope="col">Progress</th></tr></thead>
							<tbody>{#each season.episodes as episode (episode.id)}<tr>
								<td><strong>{episode.number}. {episode.title}</strong>{#if !episode.monitored}<p>Not monitored</p>{/if}</td>
								<td><Time value={episode.air_at} empty="Date unconfirmed" />{#if episode.air_at !== null && episode.air_at > now}<p>{remaining(episode.air_at)}</p>{/if}</td>
								<td><div class="episode-badges"><span class="badge badge-outline">{episode.air_at === null ? 'Air date unknown' : episode.air_at <= now ? 'Aired' : 'Upcoming'}</span><span class="badge" class:badge-success={episode.has_file} class:badge-ghost={!episode.has_file}>{episode.has_file ? 'In library' : 'Not in library'}</span>{#if episode.notified}<span class="badge badge-outline">Alert already accounted for</span>{/if}</div></td>
							</tr>{/each}</tbody>
						</table></div>
					</div>
				</details>
			{:else}<p>No episodes have been listed for this series yet.</p>{/each}
		</section>
		<footer class="page-footer"><span>Updates every 30 seconds · <Time value={detail.as_of} /></span><span>Tracking began <Time value={detail.tracking_since} /></span></footer>
	{/if}
</div>

<style>
	.progress-pair { display: grid; gap: 1rem; margin-block: .75rem; }
	.countdown-text { font-size: clamp(1.25rem, 3vw, 1.9rem); font-weight: 650; font-variant-numeric: tabular-nums; }
	.season-summary { cursor: pointer; padding: 1.25rem; display: list-item; margin-left: 1rem; }
	.season-summary span { margin-left: 1rem; }
	.episode-badges { display: flex; flex-wrap: wrap; gap: .5rem; }
	.table td { white-space: normal; min-width: 150px; }
	@media (max-width: 520px) { .season-summary span { display: block; margin: .5rem 0 0; } }
</style>
