<script lang="ts">
	import type { Show } from '$lib/api';

	let { show, disabled = false, onChange }: {
		show: Show;
		disabled?: boolean;
		onChange: (excluded: boolean) => void;
	} = $props();
</script>

<label class="flex items-center justify-between gap-2 sm:gap-4">
	<span>Track this show</span>
	<input type="checkbox" class="toggle" aria-label={`Track ${show.title}`} checked={!show.excluded}
		disabled={disabled || !show.active}
		onchange={(event) => {
			const excluded = !event.currentTarget.checked;
			event.currentTarget.checked = !show.excluded;
			if (excluded !== show.excluded) onChange(excluded);
		}} />
</label>
