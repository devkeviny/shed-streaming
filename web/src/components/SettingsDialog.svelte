<!-- Licensed under the GNU Affero General Public License v3.0 — see LICENSE. -->
<script lang="ts">
	import { api } from '$lib/api';
	import { notify } from '$lib/notify';
	import Dialog from '$lib/components/ui/dialog/Dialog.svelte';
	import { Checkbox } from '$lib/components/ui/checkbox';
	import type { OutputConfig, BroadcastConfig, AudioRouting, Destination, Source } from '$lib/types';

	type Tab = 'output' | 'stream' | 'audio' | 'browser';

	let {
		open = $bindable(false),
		initialTab = 'output',
		isLive = false,
		onSaved = () => {},
	}: {
		open?: boolean;
		initialTab?: Tab;
		isLive?: boolean;
		onSaved?: () => void;
	} = $props();

	const TABS: { id: Tab; label: string }[] = [
		{ id: 'output', label: 'Output' },
		{ id: 'stream', label: 'Stream' },
		{ id: 'audio', label: 'Audio' },
		{ id: 'browser', label: 'Browser' },
	];

	let tab = $state<Tab>(initialTab);
	let loaded = $state(false);

	// Output draft
	let draft = $state<OutputConfig>({
		video_bitrate_kbps: 4500,
		audio_bitrate_kbps: 160,
		width: 1920,
		height: 1080,
		fps: 30,
		video_encoder: 'libx264',
		egress_copy_video: false,
	});
	let resSelect = $state('1080p');
	const RES_PRESETS = [
		{ label: '2160p', w: 3840, h: 2160 },
		{ label: '1440p', w: 2560, h: 1440 },
		{ label: '1080p', w: 1920, h: 1080 },
		{ label: '720p', w: 1280, h: 720 },
		{ label: '480p', w: 854, h: 480 },
	];

	// Stream
	let destinations = $state<Destination[]>([]);
	let bcast = $state<BroadcastConfig>({
		source_id: null,
		scene_id: null,
		start_stinger_id: null,
		destination_ids: [],
		auto_record: false,
	});
	let bcastDirty = $state(false);

	// Audio
	let audioRouting = $state<AudioRouting>({
		active_audio_source: null,
		channels: [],
		audio_follows_video: true,
	});
	let sources = $state<Source[]>([]);

	// Load fresh state every time the dialog opens.
	$effect(() => {
		if (open) {
			tab = initialTab;
			loaded = false;
			bcastDirty = false;
			Promise.all([
				api.getOutputConfig().catch(() => null),
				api.listDestinations().catch(() => []),
				api.getBroadcastConfig().catch(() => null),
				api.getAudioRouting().catch(() => null),
				api.listSources().catch(() => []),
			]).then(([out, dests, bc, ar, srcs]) => {
				if (out) {
					draft = { ...out };
					const m = RES_PRESETS.find((p) => p.w === draft.width && p.h === draft.height);
					resSelect = m ? m.label : 'custom';
				}
				destinations = dests;
				if (bc) bcast = bc;
				if (ar) audioRouting = ar;
				sources = srcs;
				loaded = true;
			});
		}
	});

	function onResChange() {
		const p = RES_PRESETS.find((x) => x.label === resSelect);
		if (p) {
			draft.width = p.w;
			draft.height = p.h;
		}
	}

	async function saveOutput() {
		try {
			await api.setOutputConfig(draft);
			notify.success(isLive ? 'Output config applied' : 'Output config saved');
			onSaved();
		} catch (e) {
			notify.error(e);
		}
	}

	function toggleDestination(id: string) {
		if (bcast.destination_ids.includes(id)) {
			bcast.destination_ids = bcast.destination_ids.filter((d) => d !== id);
		} else {
			bcast.destination_ids = [...bcast.destination_ids, id];
		}
		bcastDirty = true;
	}

	async function saveStream() {
		try {
			bcast = await api.setBroadcastConfig(bcast);
			bcastDirty = false;
			notify.success('Stream config saved');
			onSaved();
		} catch (e) {
			notify.error(e);
		}
	}

	async function toggleFollowsVideo() {
		try {
			await api.toggleAudioFollowsVideo();
			audioRouting.audio_follows_video = !audioRouting.audio_follows_video;
			if (audioRouting.audio_follows_video) audioRouting.active_audio_source = null;
			onSaved();
		} catch (e) {
			notify.error(e);
		}
	}

	async function setAudioSource(sourceId: string | null) {
		try {
			await api.setAudioSource(sourceId);
			audioRouting.active_audio_source = sourceId;
			audioRouting.audio_follows_video = sourceId === null;
			onSaved();
		} catch (e) {
			notify.error(e);
		}
	}

	function close() {
		open = false;
	}
</script>

<Dialog bind:open>
	<div class="panel w-[720px] max-w-[94vw]">
		<header class="panel__head flex items-center justify-between">
			<span>▮ SETTINGS</span>
			<button onclick={close} class="text-amber-muted hover:text-amber-bright" aria-label="Close">✕</button>
		</header>
		<div class="flex min-h-[380px]">
			<!-- Category sidebar (OBS-style) -->
			<nav class="w-36 shrink-0 space-y-1 border-r border-border-dim bg-panel-raised p-2" aria-label="Settings categories">
				{#each TABS as t}
					<button
						onclick={() => (tab = t.id)}
						class="w-full rounded-sm px-3 py-2 text-left text-[12px] {tab === t.id
							? 'bg-panel text-amber-bright'
							: 'text-amber-dim hover:text-amber'}"
					>
						{t.label}
					</button>
				{/each}
			</nav>

			<!-- Category pane -->
			<div class="min-w-0 flex-1 p-4">
				{#if !loaded}
					<p class="text-amber-muted">Loading…</p>
				{:else if tab === 'output'}
					<div class="space-y-3">
						{#if isLive}
							<p class="text-[11px] text-amber-bright">You are live. Saving will briefly reconnect your destinations.</p>
						{/if}
						<div>
							<label class="field-label" for="st-res">Resolution</label>
							<select id="st-res" bind:value={resSelect} onchange={onResChange} class="select w-full">
								{#each RES_PRESETS as p}<option value={p.label}>{p.label} ({p.w}x{p.h})</option>{/each}
								<option value="custom">Custom</option>
							</select>
						</div>
						{#if resSelect === 'custom'}
							<div class="grid grid-cols-2 gap-2">
								<div>
									<label class="field-label" for="st-w">Width</label>
									<input id="st-w" type="number" min="160" max="3840" bind:value={draft.width} class="input" />
								</div>
								<div>
									<label class="field-label" for="st-h">Height</label>
									<input id="st-h" type="number" min="120" max="2160" bind:value={draft.height} class="input" />
								</div>
							</div>
						{/if}
						<div>
							<label class="field-label" for="st-fps">Frame rate</label>
							<select id="st-fps" bind:value={draft.fps} class="select w-full">
								{#each [24, 25, 30, 50, 60] as f}<option value={f}>{f} fps</option>{/each}
							</select>
						</div>
						<div>
							<label class="field-label" for="st-vb">Video bitrate (kbps)</label>
							<input id="st-vb" type="number" min="500" max="20000" step="100" bind:value={draft.video_bitrate_kbps} class="input" />
						</div>
						<div>
							<label class="field-label" for="st-ab">Audio bitrate (kbps)</label>
							<select id="st-ab" bind:value={draft.audio_bitrate_kbps} class="select w-full">
								{#each [96, 128, 160, 192, 256] as a}<option value={a}>{a} kbps</option>{/each}
							</select>
						</div>
						<div>
							<label class="field-label" for="st-enc">Video encoder</label>
							<select id="st-enc" bind:value={draft.video_encoder} class="select w-full">
								<option value="libx264">libx264 (software)</option>
								<option value="h264_v4l2m2m">h264_v4l2m2m (hardware, V4L2)</option>
							</select>
							<p class="mt-1 text-[11px] text-amber-muted">
								Use the hardware encoder on boxes with a V4L2 H.264 encoder (e.g. Amlogic S905W) — software x264 can't reach realtime at 1080p there.
							</p>
						</div>
						<label class="flex items-start gap-2 text-[12px] text-amber-dim">
							<input type="checkbox" bind:checked={draft.egress_copy_video} class="mt-0.5" />
							<span>
								Pass program video through to RTMP without re-encoding
								<span class="block text-[11px] text-amber-muted">Saves one full encode per destination. Only correct when the program comes from a scene (the compositor already outputs H.264 at this canvas).</span>
							</span>
						</label>
						<div class="flex gap-2 pt-1">
							<button onclick={saveOutput} class="btn btn--go">Save</button>
							<button onclick={close} class="btn btn--ghost">Close</button>
						</div>
					</div>
				{:else if tab === 'stream'}
					<div class="space-y-3">
						<p class="text-[12px] text-amber-dim">
							Choose which destinations receive the stream when you go live. With none selected, all enabled destinations are used.
						</p>
						{#if destinations.length === 0}
							<p class="text-amber-muted">No destinations yet. <a href="/destinations" class="text-amber hover:text-amber-bright">Add one ▸</a></p>
						{:else}
							<div class="space-y-2">
								{#each destinations as dest (dest.id)}
									<label class="flex cursor-pointer items-center gap-2">
										<Checkbox
											checked={bcast.destination_ids.length === 0
												? dest.enabled
												: bcast.destination_ids.includes(dest.id)}
											onclick={() => toggleDestination(dest.id)}
										/>
										<span class="text-amber">{dest.name}</span>
										<span class="text-[11px] text-amber-muted">{dest.enabled ? '' : '(disabled)'}</span>
									</label>
								{/each}
							</div>
						{/if}
						<div class="flex gap-2 pt-1">
							<button onclick={saveStream} class="btn btn--go" disabled={!bcastDirty}>Save</button>
							<button onclick={close} class="btn btn--ghost">Close</button>
						</div>
						<p class="text-[11px] text-amber-muted"><a href="/destinations" class="text-amber hover:text-amber-bright">Manage destinations ▸</a></p>
					</div>
				{:else if tab === 'audio'}
					<div class="space-y-3">
						<button
							onclick={toggleFollowsVideo}
							class="btn w-full {audioRouting.audio_follows_video ? 'btn--go' : ''}"
						>
							{audioRouting.audio_follows_video ? 'Audio follows video' : 'Independent audio'}
						</button>
						<p class="text-[12px] text-amber-dim">
							{#if audioRouting.audio_follows_video}
								The audio comes from whatever is on Program.
							{:else}
								Pick one source for the broadcast audio:
							{/if}
						</p>
						{#if !audioRouting.audio_follows_video}
							<div class="space-y-1">
								{#each sources.filter((s) => s.state === 'live') as s (s.id)}
									<button
										onclick={() => setAudioSource(s.id)}
										class="row w-full text-left {s.id === audioRouting.active_audio_source ? 'border-live' : ''}"
									>
										<span class="truncate {s.id === audioRouting.active_audio_source ? 'text-amber-bright' : 'text-amber-dim'}">{s.name}</span>
										{#if s.id === audioRouting.active_audio_source}
											<span class="pill pill--live ml-auto shrink-0">● ACTIVE</span>
										{/if}
									</button>
								{/each}
								{#if sources.filter((s) => s.state === 'live').length === 0}
									<p class="text-amber-muted">No live sources.</p>
								{/if}
							</div>
						{/if}
						<div class="flex gap-2 pt-1">
							<button onclick={close} class="btn btn--ghost">Close</button>
						</div>
					</div>
				{:else if tab === 'browser'}
					<div class="space-y-3 text-[12px] text-amber-dim">
						<p>
							Browser sources render a web page with headless Chrome and turn it into a video source.
							To fit this box, pages are rendered at a reduced size and frame rate, and frames are only
							captured when the page actually changes — a static page costs almost nothing.
						</p>
						<p>
							<strong class="text-amber">Tips to keep it light:</strong>
						</p>
						<ul class="list-disc space-y-1 pl-5">
							<li>Prefer static pages (overlays, alerts, scoreboards). Video-heavy sites cost far more.</li>
							<li>One browser source at a time is the safe limit on this box.</li>
							<li>A browser source takes ~40 seconds to go live the first time (Chrome starting + page loading).</li>
						</ul>
						<div class="flex gap-2 pt-1">
							<button onclick={close} class="btn btn--ghost">Close</button>
						</div>
					</div>
				{/if}
			</div>
		</div>
	</div>
</Dialog>
