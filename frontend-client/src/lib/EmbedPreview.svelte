<script lang="ts">
	import type { Snippet } from 'svelte';
	let { color, children }: { color: string; children: Snippet } = $props();
</script>

<div class="card card-border max-w-lg border-l-4 bg-base-200" style:border-left-color={color}>
	<div class="card-body gap-3 wrap-anywhere">
		<strong>Jelly Name</strong>
		{@render children()}
	</div>
</div>
