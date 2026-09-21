<script lang="ts">
	import { onMount } from 'svelte';
	import { api, errorMessage, type Mode, type Show } from '$lib/api';

	let mode = $state<Mode>('episode');
	let savedMode = $state<Mode | null>(null);
	let shows = $state<Show[]>([]);
	let loading = $state(true);
	let saving = $state(false);
	let error = $state('');
	let feedback = $state('');
	let inherited = $derived(shows.filter((show) => !show.mode_overridden).length);
	let overridden = $derived(shows.length - inherited);

	async function load() {
		loading = true;
		error = '';
		try {
			const [settings, nextShows] = await Promise.all([api<{ mode: Mode }>('settings'), api<Show[]>('shows')]);
			mode = savedMode = settings.mode;
			shows = nextShows;
		} catch (cause) { error = errorMessage(cause); }
		finally { loading = false; }
	}

	async function save() {
		saving = true;
		error = feedback = '';
		try {
			await api<void>('settings', { method: 'PUT', body: JSON.stringify({ mode }) });
			savedMode = mode;
			feedback = 'Default saved. Shows using the default now follow this preference. Individual choices are preserved.';
		} catch (cause) { error = errorMessage(cause); }
		finally { saving = false; }
	}

	async function resetAll() {
		if (saving) return;
		const selectedMode = mode;
		const label = selectedMode === 'episode' ? 'Every episode' : 'Full seasons';
		if (!window.confirm(`Reset all shows to default?

This will save “${label}” as your default and remove every show’s individual notification choice, including excluded and removed shows. All shows will follow this default and future default changes.

Excluded shows will stay excluded. Previous individual choices cannot be restored automatically.`)) return;
		saving = true;
		error = feedback = '';
		try {
			await api<void>('settings/reset-all', { method: 'PUT', body: JSON.stringify({ mode: selectedMode }) });
			mode = savedMode = selectedMode;
			shows = shows.map((show) => ({ ...show, mode: selectedMode, mode_overridden: false }));
			feedback = `All shows now use the default: ${label}. Individual notification choices have been reset.`;
		} catch (cause) { error = errorMessage(cause); }
		finally { saving = false; }
	}

	onMount(() => { void load(); });
</script>

<svelte:head><title>Settings · Jelly Alert</title></svelte:head>

<header class="section-heading page-heading">
	<div class="heading-copy"><h1 class="card-title">Settings</h1><p>Choose how your shows notify you by default.</p></div>
</header>
{#if error}<div class="alert alert-error notice" role="alert"><span>{error}</span><button class="btn btn-sm" disabled={loading || saving} onclick={load}>Reload settings</button></div>{/if}
{#if feedback}<div class="alert alert-success notice" role="status">{feedback}</div>{/if}
{#if loading}
	<p role="status">Loading settings…</p>
{:else if savedMode !== null}
	<form class="card card-border" onsubmit={(event) => { event.preventDefault(); void save(); }}>
		<div class="card-body page-stack">
			<div class="heading-copy">
				<h2 class="card-title">Default notifications</h2>
				<p>Changing this updates every show using the default, including shows added later.</p>
			</div>
			<fieldset class="page-stack" disabled={saving}>
				<legend class="fieldset-legend">Notify me</legend>
				<label class="actions"><input class="radio radio-primary" type="radio" name="mode" value="episode" bind:group={mode} /><span><strong>Every episode</strong><br />An alert when each missing episode airs.</span></label>
				<label class="actions"><input class="radio radio-primary" type="radio" name="mode" value="season" bind:group={mode} /><span><strong>Full seasons</strong><br />One alert when a full season has aired.</span></label>
			</fieldset>
			<div class="actions" aria-label="Show preferences">
				<span class="badge badge-primary badge-soft">{inherited} using default</span>
				<span class="badge badge-outline">{overridden} individual choices preserved</span>
			</div>
			<p>Individual episode or season choices stay in place, even when they match the default. Choose “Use default” on the <a class="link" href="/shows">Shows page</a> to include a show in future changes. Excluded shows stay excluded.</p>
			<div class="actions"><button class="btn btn-primary" type="submit" disabled={saving || mode === savedMode}>{saving ? 'Saving…' : 'Save default'}</button><button class="btn btn-outline btn-warning" type="button" disabled={saving} onclick={resetAll}>Reset all to default</button></div>
		</div>
	</form>
{/if}
