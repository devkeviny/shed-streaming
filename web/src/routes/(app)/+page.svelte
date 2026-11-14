<!-- Licensed under the GNU Affero General Public License v3.0 — see LICENSE. -->
<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { api } from '$lib/api';
	import {
		pipelineState,
		sources,
		destinations,
		scenes,
		isLive,
		isRecording,
		recordingState,
		failover,
	} from '$lib/stores/pipeline';
	import type { BroadcastConfig, OutputConfig, OutputStats, AudioRouting, Layer } from '$lib/types';
	import StatusIndicator from '../../components/StatusIndicator.svelte';
	import VideoPreview from '../../components/VideoPreview.svelte';
	import { popout } from '$lib/popout';
	import PopoutButton from '../../components/PopoutButton.svelte';
	import { Checkbox } from '$lib/components/ui/checkbox';
	import Dialog from '$lib/components/ui/dialog/Dialog.svelte';
	import { notify } from '$lib/notify';

	// Output config popup
	let showOutput = $state(false);
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

	let programSourceId = $state<string | null>(null);
	let previewSourceId = $state<string | null>(null);
	let audioRouting = $state<AudioRouting>({
		active_audio_source: null,
		channels: [],
		audio_follows_video: true,
	});

	/** Local Program-monitor speaker (browser only — does not change AudioRouting). */
	let programMonitorAudio = $state(false);

	// Output
	let outputConfig = $state<OutputConfig>({
		video_bitrate_kbps: 4500,
		audio_bitrate_kbps: 160,
		width: 1920,
		height: 1080,
		fps: 30,
		video_encoder: 'libx264',
		egress_copy_video: false,
	});
	let outputStats = $state<OutputStats>({
		bytes_sent: 0,
		duration_secs: 0,
		source_bitrate_kbps: 0,
		output_bitrate_kbps: 0,
		dropped_frames: 0,
	});

	// Broadcast config
	let config = $state<BroadcastConfig>({
		source_id: null,
		scene_id: null,
		start_stinger_id: null,
		destination_ids: [],
		auto_record: false,
	});
	let configDirty = $state(false);
	let statsInterval: ReturnType<typeof setInterval> | null = null;

	// OBS-style dock state: the selected scene drives the Sources panel,
	// which lists that scene's layers (not the global source list).
	let selectedSceneId = $state<string | null>(null);
	let selectedLayerId = $state<string | null>(null);
	let addSourceId = $state('');
	let newSceneName = $state('');
	let cutSourceId = $state('');

	const selectedScene = $derived($scenes.find((s) => s.id === selectedSceneId) ?? null);
	const orderedLayers = $derived(
		selectedScene ? [...selectedScene.layers].sort((a, b) => a.z_index - b.z_index) : []
	);
	const selectedLayer = $derived(
		selectedScene?.layers.find((l) => l.id === selectedLayerId) ?? null
	);

	function srcName(id: string): string {
		return $sources.find((s) => s.id === id)?.name ?? id.slice(0, 8);
	}

	function srcKind(id: string): string {
		return $sources.find((s) => s.id === id)?.kind.type ?? '';
	}

	async function refreshScenes() {
		const list = await api.listScenes();
		scenes.set(list);
		if (selectedSceneId && !list.find((s) => s.id === selectedSceneId)) {
			selectedSceneId = list[0]?.id ?? null;
			selectedLayerId = null;
		}
	}

	async function createSceneInline() {
		if (!newSceneName.trim()) return;
		try {
			const s = await api.createScene(newSceneName.trim());
			newSceneName = '';
			await refreshScenes();
			selectedSceneId = s.id;
			notify.success('Scene created');
		} catch (e) {
			notify.error(e);
		}
	}

	async function duplicateSceneFn(id: string) {
		try {
			const s = await api.duplicateScene(id);
			await refreshScenes();
			selectedSceneId = s.id;
			notify.success('Scene duplicated');
		} catch (e) {
			notify.error(e);
		}
	}

	async function deleteSceneFn(id: string) {
		try {
			await api.deleteScene(id);
			if (selectedSceneId === id) {
				selectedSceneId = null;
				selectedLayerId = null;
			}
			await refreshScenes();
		} catch (e) {
			notify.error(e);
		}
	}

	async function addLayerToSelected() {
		if (!selectedScene || !addSourceId) return;
		try {
			const n = selectedScene.layers.length;
			const layer =
				n === 0
					? { source_id: addSourceId, x: 0, y: 0, width: outputConfig.width, height: outputConfig.height, z_index: 0, opacity: 1 }
					: { source_id: addSourceId, x: Math.round(outputConfig.width * 0.6), y: Math.round(outputConfig.height * 0.57), width: Math.round(outputConfig.width / 3), height: Math.round(outputConfig.height / 3), z_index: n, opacity: 1 };
			const l = await api.addLayer(selectedScene.id, layer);
			addSourceId = '';
			await refreshScenes();
			selectedLayerId = l.id;
		} catch (e) {
			notify.error(e);
		}
	}

	async function removeLayerFn(layerId: string) {
		if (!selectedScene) return;
		try {
			await api.deleteLayer(selectedScene.id, layerId);
			if (selectedLayerId === layerId) selectedLayerId = null;
			await refreshScenes();
		} catch (e) {
			notify.error(e);
		}
	}

	async function saveLayerFn(l: Layer) {
		if (!selectedScene) return;
		try {
			await api.updateLayer(selectedScene.id, l.id, {
				x: Math.round(l.position.x),
				y: Math.round(l.position.y),
				width: Math.round(l.size.width),
				height: Math.round(l.size.height),
				z_index: l.z_index,
				opacity: l.opacity,
				fit: l.fit,
				visible: l.visible,
			});
		} catch (e) {
			notify.error(e);
		}
	}

	async function toggleLayerVisible(l: Layer) {
		l.visible = !l.visible;
		await saveLayerFn(l);
		await refreshScenes();
	}

	async function moveLayerZ(l: Layer, dir: 1 | -1) {
		l.z_index = Math.max(0, l.z_index + dir);
		await saveLayerFn(l);
		await refreshScenes();
	}

	onMount(async () => {
		const [srcList, destList, sceneList, status, recStatus, savedConfig, outConfig] =
			await Promise.all([
				api.listSources(),
				api.listDestinations(),
				api.listScenes(),
				api.getStatus(),
				api.recordingStatus().catch(() => ({ recording: false })),
				api.getBroadcastConfig().catch(() => null),
				api.getOutputConfig().catch(() => null),
			]);
		sources.set(srcList);
		destinations.set(destList);
		scenes.set(sceneList);
		pipelineState.set(status.pipeline);
		recordingState.set(recStatus);
		if (savedConfig) config = savedConfig;
		if (outConfig) outputConfig = outConfig;
		// Select the live scene, else the first scene.
		selectedSceneId = status.pipeline.active_scene ?? sceneList[0]?.id ?? null;

		audioRouting = await api.getAudioRouting().catch(() => audioRouting);
		if (status.pipeline.state === 'live') {
			const prog = await api.getProgram().catch(() => null);
			if (prog) {
				programSourceId = prog.program_source_id;
				previewSourceId = prog.preview_source_id;
			}
			startStatsPolling();
		}
	});

	let studioChannel: BroadcastChannel;

	onMount(() => {
		studioChannel = new BroadcastChannel('muxshed-studio');
		studioChannel.onmessage = (e) => {
			if (e.data.type === 'cut_source') cutToSource(e.data.sourceId);
			if (e.data.type === 'set_preview') previewSourceId = e.data.sourceId;
			if (e.data.type === 'audio_routing') audioRouting = e.data.routing;
			if (e.data.type === 'request_state') broadcastState();
		};
		return () => studioChannel?.close();
	});

	function broadcastState() {
		studioChannel?.postMessage({ type: 'program_source', sourceId: programSourceId });
		studioChannel?.postMessage({ type: 'preview_source', sourceId: previewSourceId });
		studioChannel?.postMessage({ type: 'audio_routing', routing: audioRouting });
	}

	// Sync popout windows when state changes
	$effect(() => {
		studioChannel?.postMessage({ type: 'program_source', sourceId: programSourceId });
	});
	$effect(() => {
		studioChannel?.postMessage({ type: 'preview_source', sourceId: previewSourceId });
	});

	onDestroy(() => {
		if (statsInterval) clearInterval(statsInterval);
		studioChannel?.close();
	});

	function startStatsPolling() {
		if (statsInterval) clearInterval(statsInterval);
		statsInterval = setInterval(async () => {
			outputStats = await api.getOutputStats().catch(() => outputStats);
		}, 2000);
	}

	function stopStatsPolling() {
		if (statsInterval) {
			clearInterval(statsInterval);
			statsInterval = null;
		}
	}

	function liveSources() {
		return $sources.filter((s) => s.state === 'live');
	}

	async function cutToSource(id: string) {
		try {
			await api.cutToSource(id);
			programSourceId = id;
		} catch (e) {
			notify.error(e);
		}
	}

	async function takeScene(id: string) {
		try {
			await api.activateScene(id);
			programSourceId = id;
			previewSourceId = null;
			notify.success('Scene is live');
		} catch (e) {
			notify.error(e);
		}
	}

	async function pushPreviewToLive() {
		if (!previewSourceId) {
			notify.error('No source queued in Next Up');
			return;
		}
		try {
			await api.cutToSource(previewSourceId);
			programSourceId = previewSourceId;
			previewSourceId = null;
		} catch (e) {
			notify.error(e);
		}
	}

	async function setAudioSource(sourceId: string | null) {
		try {
			await api.setAudioSource(sourceId);
			audioRouting.active_audio_source = sourceId;
			audioRouting.audio_follows_video = sourceId === null;
		} catch (e) {
			notify.error(e);
		}
	}

	async function toggleRecording() {
		try {
			if ($isRecording) {
				await api.stopRecording();
			} else {
				await api.startRecording();
			}
		} catch (e) {
			notify.error(e);
		}
	}

	async function saveConfig() {
		try {
			config = await api.setBroadcastConfig(config);
			configDirty = false;
			notify.success('Config saved');
		} catch (e) {
			notify.error(e);
		}
	}

	async function saveOutputConfig() {
		try {
			outputConfig = await api.setOutputConfig(outputConfig);
		} catch (e) {
			notify.error(e);
		}
	}

	function openOutput() {
		draft = { ...outputConfig };
		const m = RES_PRESETS.find((p) => p.w === draft.width && p.h === draft.height);
		resSelect = m ? m.label : 'custom';
		showOutput = true;
	}

	function onResChange() {
		const p = RES_PRESETS.find((x) => x.label === resSelect);
		if (p) {
			draft.width = p.w;
			draft.height = p.h;
		}
	}

	async function saveOutput() {
		try {
			outputConfig = await api.setOutputConfig(draft);
			showOutput = false;
			notify.success($isLive ? 'Output config applied' : 'Output config saved');
		} catch (e) {
			notify.error(e);
		}
	}

	function formatBytes(bytes: number): string {
		if (bytes < 1024) return `${bytes} B`;
		if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} KB`;
		if (bytes < 1073741824) return `${(bytes / 1048576).toFixed(1)} MB`;
		return `${(bytes / 1073741824).toFixed(2)} GB`;
	}

	function formatDuration(secs: number): string {
		const h = Math.floor(secs / 3600);
		const m = Math.floor((secs % 3600) / 60);
		const s = Math.floor(secs % 60);
		return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
	}

	function toggleDestination(id: string) {
		if (config.destination_ids.includes(id)) {
			config.destination_ids = config.destination_ids.filter((d) => d !== id);
		} else {
			config.destination_ids = [...config.destination_ids, id];
		}
		configDirty = true;
	}

	// After go live succeeds, start polling stats
	$effect(() => {
		if ($isLive) {
			startStatsPolling();
			api.getProgram().then((p) => {
				if (p) {
					programSourceId = p.program_source_id;
					previewSourceId = p.preview_source_id;
				}
			}).catch(() => {});
		} else {
			stopStatsPolling();
		}
	});
</script>

<div class="mx-auto max-w-[1400px]">
	{#if $failover.active}
		{@const fbName = $sources.find((s) => s.id === $failover.fallback_source_id)?.name ?? 'fallback'}
		{@const mainName = $sources.find((s) => s.id === $failover.intent_source_id)?.name ?? 'your source'}
		<div class="mb-3 flex items-center gap-3 rounded border border-warning bg-warning/10 px-4 py-2 text-warning" role="status">
			<span class="led text-[16px] animate-pulse">⚠ FAILOVER ACTIVE</span>
			<span class="text-xs">{mainName} is offline — broadcasting <strong>{fbName}</strong>. Will switch back automatically when it returns.</span>
		</div>
	{/if}

	<!-- Header -->
	<div class="mb-4 flex items-center justify-between border-b border-border pb-3">
		<h1 class="text-[13px] tracking-widest text-amber-bright">STUDIO</h1>
		<div class="flex items-center gap-3">
			<StatusIndicator state={$pipelineState} />
			{#if $isLive && $pipelineState.state === 'live'}
				<span class="led text-[18px]">{formatDuration(outputStats.duration_secs)}</span>
			{/if}
		</div>
	</div>

	<!-- Preview + Program monitors (Studio Mode: Preview left, Program right) -->
	<div class="mb-4 grid grid-cols-1 gap-4 lg:grid-cols-2">
		<!-- Preview / Next Up (left) -->
		<section class="panel">
			<header class="panel__head">
				<span class="flex items-center gap-2">
					▮ PREVIEW
					<PopoutButton section="preview" width={640} height={420} />
				</span>
				{#if previewSourceId}
					<button onclick={pushPreviewToLive} class="btn btn--danger" style="min-height:24px;padding:2px 10px">
						▸ Push to Live
					</button>
				{/if}
			</header>
			{#if previewSourceId}
				{#key previewSourceId}
					<VideoPreview sourceId={previewSourceId} />
				{/key}
			{:else}
				<div class="scanlines-well flex aspect-video items-center justify-center border-t border-border">
					<span class="text-amber-muted">Select a source below to queue</span>
				</div>
			{/if}
		</section>

		<!-- Program / Live (right) -->
		<section class="panel">
			<header class="panel__head">
				<span class="flex items-center gap-2 {$isLive ? 'text-danger-glow' : ''}">
					▮ PROGRAM
					<PopoutButton section="program" width={960} height={600} />
				</span>
				<div class="flex items-center gap-2">
					{#if programSourceId}
						{@const progSrc = $sources.find((s) => s.id === programSourceId)}
						{@const progScene = $scenes.find((s) => s.id === programSourceId)}
						<span class="text-amber-dim normal-case tracking-normal">{progScene?.name || progSrc?.name || ''}</span>
					{/if}
					<button
						type="button"
						class="btn {programMonitorAudio ? 'btn--go' : ''}"
						style="min-height:24px;padding:2px 8px"
						title="Local speaker monitor only — use headphones to avoid feedback. Does not change broadcast audio."
						onclick={() => (programMonitorAudio = !programMonitorAudio)}
					>
						{programMonitorAudio ? '▮ Monitor Audio' : '▯ Monitor Audio'}
					</button>
				</div>
			</header>
			{#if programSourceId}
				{#key programSourceId}
					<VideoPreview
						sourceId={programSourceId}
						active={true}
						monitorAudio={programMonitorAudio}
					/>
				{/key}
				<p class="border-t border-border-dim px-3 py-1 text-[11px] text-amber-muted">
					Monitor Audio is local to this browser only. Prefer headphones to avoid acoustic feedback.
				</p>
			{:else}
				<div class="scanlines-well flex aspect-video items-center justify-center border-t border-border">
					<span class="text-amber-muted">No source on live</span>
				</div>
			{/if}
		</section>
	</div>

	<!-- ========== OBS DOCKS: Scenes | Sources | Audio Mixer | Controls ========== -->
	<div class="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-12">
		<!-- Scenes -->
		<section class="panel xl:col-span-3">
			<header class="panel__head">▮ SCENES</header>
			<div class="panel__body space-y-2">
				{#if $scenes.length === 0}
					<p class="text-amber-muted">No scenes yet.</p>
				{:else}
					<div class="space-y-1">
						{#each $scenes as scene (scene.id)}
							<div
								class="flex items-center gap-2 rounded-sm border px-2 py-1.5 {scene.id === selectedSceneId
									? 'border-amber bg-panel-raised'
									: scene.id === programSourceId
										? 'border-danger bg-panel-raised'
										: 'border-border-dim hover:border-border'}"
							>
								<button
									onclick={() => { selectedSceneId = scene.id; selectedLayerId = null; }}
									class="min-w-0 flex-1 truncate text-left text-amber"
									title="Select scene to edit its sources"
								>
									{scene.name}
								</button>
								<span class="shrink-0 text-[10px] text-amber-muted">{scene.layers.length} layer{scene.layers.length !== 1 ? 's' : ''}</span>
								{#if scene.id === programSourceId}
									<span class="pill pill--live shrink-0">● LIVE</span>
								{:else}
									<button
										onclick={() => takeScene(scene.id)}
										class="btn btn--danger shrink-0"
										style="min-height:22px;padding:1px 8px;font-size:11px"
										title="Take this scene to Program"
									>
										Take
									</button>
								{/if}
								<button
									onclick={() => duplicateSceneFn(scene.id)}
									class="btn btn--ghost shrink-0"
									style="min-height:22px;padding:1px 6px;font-size:11px"
									title="Duplicate scene"
								>
									⧉
								</button>
								<button
									onclick={() => deleteSceneFn(scene.id)}
									class="btn btn--danger shrink-0"
									style="min-height:22px;padding:1px 6px;font-size:11px"
									title="Delete scene"
								>
									✕
								</button>
							</div>
						{/each}
					</div>
				{/if}
				<form
					onsubmit={(e) => { e.preventDefault(); createSceneInline(); }}
					class="flex gap-2 border-t border-border-dim pt-2"
				>
					<input
						bind:value={newSceneName}
						placeholder="New scene name"
						class="input flex-1"
						aria-label="New scene name"
					/>
					<button type="submit" class="btn" disabled={!newSceneName.trim()}>+ Add</button>
				</form>
				<p class="text-[11px] text-amber-muted">
					<a href="/scenes" class="text-amber hover:text-amber-bright">Canvas editor ▸</a>
					<span class="mx-1">·</span>
					<a href="/sources" class="text-amber hover:text-amber-bright">Manage sources ▸</a>
				</p>
			</div>
		</section>

		<!-- Sources (layers of the selected scene) -->
		<section class="panel xl:col-span-4">
			<header class="panel__head">
				<span>▮ SOURCES{#if selectedScene} — {selectedScene.name}{/if}</span>
			</header>
			<div class="panel__body space-y-2">
				{#if !selectedScene}
					<p class="text-amber-muted">Select a scene to see its sources.</p>
				{:else if orderedLayers.length === 0}
					<p class="text-amber-muted">Empty scene — add a source below.</p>
				{:else}
					<div class="space-y-1">
						{#each orderedLayers as l (l.id)}
							<div
								class="flex items-center gap-2 rounded-sm border px-2 py-1.5 {l.id === selectedLayerId
									? 'border-amber bg-panel-raised'
									: 'border-border-dim hover:border-border'} {l.visible === false ? 'opacity-50' : ''}"
							>
								<button
									onclick={() => toggleLayerVisible(l)}
									class="shrink-0 text-amber-dim hover:text-amber-bright"
									title={l.visible === false ? 'Show layer' : 'Hide layer'}
									aria-label={l.visible === false ? 'Show layer' : 'Hide layer'}
								>
									{l.visible === false ? '🚫' : '👁'}
								</button>
								<button
									onclick={() => { selectedLayerId = l.id; }}
									class="min-w-0 flex-1 truncate text-left text-amber"
									title="Select layer to edit its position and size"
								>
									{srcName(l.source_id)}
								</button>
								<span class="shrink-0 text-[10px] uppercase tracking-wide text-amber-muted">{srcKind(l.source_id)}</span>
								<button
									onclick={() => moveLayerZ(l, 1)}
									class="btn btn--ghost shrink-0"
									style="min-height:22px;padding:1px 6px;font-size:11px"
									title="Bring forward"
								>
									▲
								</button>
								<button
									onclick={() => moveLayerZ(l, -1)}
									class="btn btn--ghost shrink-0"
									style="min-height:22px;padding:1px 6px;font-size:11px"
									title="Send backward"
								>
									▼
								</button>
								<button
									onclick={() => removeLayerFn(l.id)}
									class="btn btn--danger shrink-0"
									style="min-height:22px;padding:1px 6px;font-size:11px"
									title="Remove layer from scene"
								>
									✕
								</button>
							</div>
						{/each}
					</div>
				{/if}
				{#if selectedScene}
					<div class="flex gap-2 border-t border-border-dim pt-2">
						<select bind:value={addSourceId} class="select flex-1" aria-label="Source to add">
							<option value="">Add source to scene…</option>
							{#each $sources as s}
								<option value={s.id}>{s.name} ({s.kind.type})</option>
							{/each}
						</select>
						<button onclick={addLayerToSelected} disabled={!addSourceId} class="btn">+ Add</button>
					</div>
				{/if}
				{#if selectedLayer}
					<div class="space-y-2 border-t border-border-dim pt-2">
						<div class="label">Selected layer — {srcName(selectedLayer.source_id)}</div>
						<div class="grid grid-cols-4 gap-2">
							<div>
								<label class="field-label" for="ly-x">X</label>
								<input id="ly-x" type="number" bind:value={selectedLayer.position.x} onchange={() => saveLayerFn(selectedLayer!)} class="input" />
							</div>
							<div>
								<label class="field-label" for="ly-y">Y</label>
								<input id="ly-y" type="number" bind:value={selectedLayer.position.y} onchange={() => saveLayerFn(selectedLayer!)} class="input" />
							</div>
							<div>
								<label class="field-label" for="ly-w">W</label>
								<input id="ly-w" type="number" bind:value={selectedLayer.size.width} onchange={() => saveLayerFn(selectedLayer!)} class="input" />
							</div>
							<div>
								<label class="field-label" for="ly-h">H</label>
								<input id="ly-h" type="number" bind:value={selectedLayer.size.height} onchange={() => saveLayerFn(selectedLayer!)} class="input" />
							</div>
						</div>
						<div class="flex items-center gap-2">
							<label class="field-label" for="ly-op">Opacity</label>
							<input
								id="ly-op"
								type="range"
								min="0"
								max="1"
								step="0.05"
								bind:value={selectedLayer.opacity}
								onchange={() => saveLayerFn(selectedLayer!)}
								class="flex-1"
							/>
							<span class="w-10 text-right text-amber">{Math.round(selectedLayer.opacity * 100)}%</span>
						</div>
					</div>
				{/if}
			</div>
		</section>

		<!-- Audio Mixer -->
		<section class="panel xl:col-span-2">
			<header class="panel__head">
				<span class="flex items-center gap-2">
					▮ AUDIO MIXER
					<PopoutButton section="audio" width={400} height={500} />
				</span>
			</header>
			<div class="panel__body space-y-2">
				<button
					onclick={async () => {
						try {
							await api.toggleAudioFollowsVideo();
							audioRouting.audio_follows_video = !audioRouting.audio_follows_video;
							if (audioRouting.audio_follows_video) audioRouting.active_audio_source = null;
						} catch (e) { notify.error(e); }
					}}
					class="btn w-full {audioRouting.audio_follows_video ? 'btn--go' : ''}"
				>
					{audioRouting.audio_follows_video ? 'Follows Video' : 'Independent'}
				</button>
				{#if liveSources().length === 0}
					<p class="text-amber-muted">No live sources.</p>
				{:else}
					<div class="space-y-1">
						{#each liveSources() as source (source.id)}
							{@const isAudioSource = audioRouting.audio_follows_video
								? source.id === programSourceId
								: source.id === audioRouting.active_audio_source}
							<button
								onclick={() => setAudioSource(source.id)}
								disabled={audioRouting.audio_follows_video}
								class="row w-full text-left {isAudioSource ? 'border-live' : ''} disabled:cursor-default"
							>
								<div class="min-w-0 flex-1">
									<div class="truncate {isAudioSource ? 'text-amber-bright' : 'text-amber-dim'}">
										{source.name}
									</div>
								</div>
								{#if isAudioSource}
									<span class="pill pill--live shrink-0">● ACTIVE</span>
								{/if}
							</button>
						{/each}
					</div>
				{/if}
				<p class="text-[11px] text-amber-muted">
					{#if audioRouting.audio_follows_video}
						Audio follows the video on Program.
					{:else if audioRouting.active_audio_source}
						{@const audioSrc = $sources.find((s) => s.id === audioRouting.active_audio_source)}
						Audio: {audioSrc?.name || 'Unknown'}
					{:else}
						No audio source selected.
					{/if}
				</p>
			</div>
		</section>

		<!-- Controls -->
		<section class="panel xl:col-span-3">
			<header class="panel__head">▮ CONTROLS</header>
			<div class="panel__body space-y-3">
				{#if $isLive}
					<button
						onclick={async () => { try { await api.stopStream(); } catch(e) { notify.error(e); }}}
						class="btn btn--danger w-full"
					>
						■ End Stream
					</button>
					<button onclick={toggleRecording} class="btn w-full {$isRecording ? 'btn--danger' : ''}">
						{$isRecording ? '● Stop Rec' : '● Record'}
					</button>
					{#if $pipelineState.state === 'live'}
						<span class="pill pill--live">● ON AIR — since {new Date($pipelineState.started_at).toLocaleTimeString()}</span>
					{/if}
				{:else}
					<button
						onclick={async () => {
							try {
								await api.startStream(programSourceId || undefined);
								startStatsPolling();
							} catch(e) {
								notify.error(e);
							}
						}}
						class="btn btn--go w-full"
					>
						● Go Live
					</button>
					<span class="pill pill--idle">○ OFF AIR</span>
				{/if}

				<div class="border-t border-border-dim pt-2">
					<span class="label">Cut to source (no scene)</span>
					<div class="mt-1 flex gap-2">
						<select bind:value={cutSourceId} class="select flex-1" aria-label="Source to cut to">
							<option value="">Raw source…</option>
							{#each liveSources() as s}
								<option value={s.id}>{s.name}</option>
							{/each}
						</select>
						<button onclick={() => cutToSource(cutSourceId)} disabled={!cutSourceId} class="btn btn--danger">Cut</button>
					</div>
				</div>

				<div class="space-y-1 border-t border-border-dim pt-2">
					<div class="flex items-center justify-between">
						<span class="label">Output</span>
						<button
							onclick={openOutput}
							class="btn btn--ghost"
							style="min-height:20px;padding:1px 8px;font-size:11px"
							aria-label="Edit output config">Edit</button
						>
					</div>
					<div class="flex justify-between">
						<span class="text-amber-dim">Resolution</span>
						<span class="text-amber-bright">{outputConfig.width}x{outputConfig.height}@{outputConfig.fps}fps</span>
					</div>
					<div class="flex justify-between">
						<span class="text-amber-dim">Video bitrate</span>
						<span class="text-amber-bright">{outputConfig.video_bitrate_kbps} kbps</span>
					</div>
					{#if outputStats.source_encoder}
						<div class="flex justify-between">
							<span class="text-amber-dim">Encoder</span>
							<span class="ml-2 truncate text-amber-dim">{outputStats.source_encoder}</span>
						</div>
					{/if}
					{#if $destinations.length > 0}
						<div class="mt-2 border-t border-border-dim pt-2">
							<span class="label">Destinations</span>
							<div class="mt-1 space-y-1">
								{#each $destinations as dest (dest.id)}
									<button
										onclick={() => toggleDestination(dest.id)}
										class="flex cursor-pointer items-center gap-2"
									>
										<Checkbox
											checked={config.destination_ids.length === 0
												? dest.enabled
												: config.destination_ids.includes(dest.id)}
										/>
										<span class="text-amber">{dest.name}</span>
									</button>
								{/each}
							</div>
						</div>
					{:else}
						<div class="flex justify-between">
							<span class="text-amber-dim">Destinations</span>
							<a href="/destinations" class="text-amber hover:text-amber-bright">Add one ▸</a>
						</div>
					{/if}
					{#if configDirty}
						<button onclick={saveConfig} class="btn mt-2 w-full">Save Config</button>
					{/if}
				</div>

				<div class="space-y-1 border-t border-border-dim pt-2">
					<span class="label">Stream stats</span>
					<div class="flex items-baseline justify-between">
						<span class="text-amber-dim">Duration</span>
						<span class="led text-[18px]">{formatDuration(outputStats.duration_secs)}</span>
					</div>
					<div class="flex justify-between">
						<span class="text-amber-dim">Data sent</span>
						<span class="text-amber-bright">{formatBytes(outputStats.bytes_sent)}</span>
					</div>
					<div class="flex justify-between">
						<span class="text-amber-dim">Source</span>
						<span class="text-amber-bright">
							{outputStats.source_width || '?'}x{outputStats.source_height || '?'}
							{#if outputStats.source_fps}@{outputStats.source_fps.toFixed(0)}fps{/if}
						</span>
					</div>
					<div class="flex items-baseline justify-between">
						<span class="text-amber-dim">Output bitrate</span>
						<span class="led text-[18px]">{outputStats.output_bitrate_kbps} kbps</span>
					</div>
					<div class="flex justify-between">
						<span class="text-amber-dim">Dropped frames</span>
						<span class="text-amber-bright">{outputStats.dropped_frames}</span>
					</div>
				</div>
			</div>
		</section>
	</div>
</div>

<Dialog bind:open={showOutput}>
	<div class="panel w-[380px] max-w-[90vw]">
		<header class="panel__head flex items-center justify-between">
			<span>▮ OUTPUT CONFIG</span>
			<button onclick={() => (showOutput = false)} class="text-amber-muted hover:text-amber-bright" aria-label="Close">✕</button>
		</header>
		<div class="panel__body space-y-3">
			{#if $isLive}
				<p class="text-[11px] text-amber-bright">You are live. Saving will briefly reconnect your destinations.</p>
			{/if}
			<div>
				<label class="field-label" for="oc-res">Resolution</label>
				<select id="oc-res" bind:value={resSelect} onchange={onResChange} class="select w-full">
					{#each RES_PRESETS as p}<option value={p.label}>{p.label} ({p.w}x{p.h})</option>{/each}
					<option value="custom">Custom</option>
				</select>
			</div>
			{#if resSelect === 'custom'}
				<div class="grid grid-cols-2 gap-2">
					<div>
						<label class="field-label" for="oc-w">Width</label>
						<input id="oc-w" type="number" min="160" max="3840" bind:value={draft.width} class="input" />
					</div>
					<div>
						<label class="field-label" for="oc-h">Height</label>
						<input id="oc-h" type="number" min="120" max="2160" bind:value={draft.height} class="input" />
					</div>
				</div>
			{/if}
			<div>
				<label class="field-label" for="oc-fps">Frame rate</label>
				<select id="oc-fps" bind:value={draft.fps} class="select w-full">
					{#each [24, 25, 30, 50, 60] as f}<option value={f}>{f} fps</option>{/each}
				</select>
			</div>
			<div>
				<label class="field-label" for="oc-vb">Video bitrate (kbps)</label>
				<input id="oc-vb" type="number" min="500" max="20000" step="100" bind:value={draft.video_bitrate_kbps} class="input" />
			</div>
			<div>
				<label class="field-label" for="oc-ab">Audio bitrate (kbps)</label>
				<select id="oc-ab" bind:value={draft.audio_bitrate_kbps} class="select w-full">
					{#each [96, 128, 160, 192, 256] as a}<option value={a}>{a} kbps</option>{/each}
				</select>
			</div>
			<div>
				<label class="field-label" for="oc-enc">Video encoder</label>
				<select id="oc-enc" bind:value={draft.video_encoder} class="select w-full">
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
				<button onclick={() => (showOutput = false)} class="btn btn--ghost">Cancel</button>
			</div>
		</div>
	</div>
</Dialog>
