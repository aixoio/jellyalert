<script lang="ts">
    let { id }: { id: number } = $props();
    let failed = $state(false);
</script>

<figure class="show-poster">
    {#if failed}
        <div class="poster-placeholder" aria-label="Artwork unavailable">
            <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><rect x="3" y="5" width="18" height="14" rx="2"/><path d="m9 10 5 3-5 3Z"/></svg>
            <span>No artwork</span>
        </div>
    {:else}
        <img src={`/api/shows/${id}/poster`} alt="" width="300" height="450" loading="lazy" decoding="async" onerror={() => failed = true} />
    {/if}
</figure>
