<script lang="ts">
	import type { Show, ShowMode } from '$lib/api';

	let { show, disabled = false, savedMode, onChange }: {
		show: Show;
		disabled?: boolean;
		savedMode?: ShowMode;
		onChange: (mode: ShowMode) => void;
	} = $props();
	let selected = $derived(savedMode ?? (show.mode_overridden ? show.mode : 'default'));
</script>

<label class="fieldset">
	<span class="fieldset-legend">Notify me</span>
	<select class="select w-full" aria-label={`Notification mode for ${show.title}`} value={selected}
		disabled={disabled || !show.active}
		onchange={(event) => {
			const mode = event.currentTarget.value;
			event.currentTarget.value = selected;
			if (mode === selected) return;
			if (mode === 'default' || mode === 'episode' || mode === 'season') onChange(mode);
		}}>
		<option value="default">Use default{!show.mode_overridden && savedMode === undefined ? ` · ${show.mode === 'episode' ? 'Every episode' : 'Full seasons'}` : ''}</option>
		<option value="episode">Every episode</option>
		<option value="season">Full seasons</option>
	</select>
</label>
