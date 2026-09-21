<script lang="ts">
    let { id, fullWidth = false, compact = false }: { id: number; fullWidth?: boolean; compact?: boolean } = $props();
    let failed = $state(false);
</script>

<figure class={fullWidth ? "absolute inset-0 size-full overflow-hidden bg-base-200" : compact ? "aspect-2/3 w-10 shrink-0 overflow-hidden rounded-box bg-base-200" : "aspect-2/3 w-20 shrink-0 overflow-hidden rounded-box bg-base-200"}>
    {#if failed}
        <div class="grid size-full place-content-center" aria-label="Artwork unavailable">
            <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><rect x="3" y="5" width="18" height="14" rx="2"/><path d="m9 10 5 3-5 3Z"/></svg>
        </div>
    {:else}
        <img class="size-full object-cover" src={`/api/shows/${id}/poster`} alt="" width="80" height="120" loading="lazy" decoding="async" onerror={() => failed = true} />
    {/if}
</figure>
