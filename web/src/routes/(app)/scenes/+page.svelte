<!-- Licensed under the GNU Affero General Public License v3.0 — see LICENSE. -->
<script lang="ts">
	import { onMount } from 'svelte';
	import { api } from '$lib/api';
	import { scenes, sources, pipelineState } from '$lib/stores/pipeline';
	import { notify } from '$lib/notify';
	import type { Scene, Layer, Asset } from '$lib/types';

	const OUT_W = 1920;
	const OUT_H = 1080;

	let name = $state('');
	let creating = $state(false);
	let editing = $state<Scene | null>(null);
	let selectedLayer = $state<string | null>(null);
	let addSourceId = $state('');
	let canvasEl = $state<HTMLDivElement | null>(null);
	let assets = $state<Asset[]>([]);

	// Canvas size follows the output config (the editor used to assume 1080p).
	let outW = $state(OUT_W);
	let outH = $state(OUT_H);

	// Scene presets for the create form.
	type Preset = 'blank' | 'fullscreen' | 'pip' | 'split';
	let preset = $state<Preset>('blank');
	let presetSourceA = $state('');
	let presetSourceB = $state('');

	onMount(async () => {
		await refresh();
		sources.set(await api.listSources());
		assets = await api.listAssets();
		const cfg = await api.getOutputConfig().catch(() => null);
		if (cfg) {
			outW = cfg.width;
			outH = cfg.height;
		}
	});

	async function refresh() {
		const list = await api.listScenes();
		scenes.set(list);
		editing = editing ? (list.find((s) => s.id === editing!.id) ?? null) : (list[0] ?? null);
	}

	const activeId = $derived($pipelineState.state === 'live' ? $pipelineState.active_scene : null);

	const orderedLayers = $derived(
		editing ? [...editing.layers].sort((a, b) => a.z_index - b.z_index) : []
	);
	const selected = $derived(editing?.layers.find((l) => l.id === selectedLayer) ?? null);

	function srcName(id: string): string {
		return $sources.find((s) => s.id === id)?.name ?? id.slice(0, 8);
	}

	async function create() {
		if (!name.trim()) return;
		creating = true;
		try {
			// Resolve a media-library asset to a source id first (same as addLayer).
			async function resolveSource(id: string): Promise<string> {
				if (id.startsWith('asset:')) {
					const src = await api.createSourceFromAsset(id.slice(6));
					sources.set(await api.listSources());
					return src.id;
				}
				return id;
			}
			let layers;
			if (preset === 'fullscreen' && presetSourceA) {
				layers = [
					{ source_id: await resolveSource(presetSourceA), x: 0, y: 0, width: outW, height: outH, z_index: 0, opacity: 1 },
				];
			} else if (preset === 'pip' && presetSourceA && presetSourceB) {
				const pw = Math.round(outW / 3);
				const ph = Math.round(outH / 3);
				layers = [
					{ source_id: await resolveSource(presetSourceA), x: 0, y: 0, width: outW, height: outH, z_index: 0, opacity: 1 },
					{ source_id: await resolveSource(presetSourceB), x: outW - pw - 24, y: outH - ph - 24, width: pw, height: ph, z_index: 1, opacity: 1 },
				];
			} else if (preset === 'split' && presetSourceA && presetSourceB) {
				layers = [
					{ source_id: await resolveSource(presetSourceA), x: 0, y: 0, width: Math.floor(outW / 2), height: outH, z_index: 0, opacity: 1 },
					{ source_id: await resolveSource(presetSourceB), x: Math.ceil(outW / 2), y: 0, width: Math.floor(outW / 2), height: outH, z_index: 1, opacity: 1 },
				];
			}
			const s = await api.createScene(name.trim(), layers);
			name = '';
			preset = 'blank';
			presetSourceA = '';
			presetSourceB = '';
			await refresh();
			editing = $scenes.find((x) => x.id === s.id) ?? null;
			notify.success('Scene created');
		} catch (e) {
			notify.error(e);
		} finally {
			creating = false;
		}
	}

	async function duplicate(id: string) {
		try {
			const s = await api.duplicateScene(id);
			await refresh();
			editing = $scenes.find((x) => x.id === s.id) ?? null;
			notify.success('Scene duplicated');
		} catch (e) {
			notify.error(e);
		}
	}

	async function activate(id: string) {
		try {
			await api.activateScene(id);
			notify.success('Scene is live');
		} catch (e) {
			notify.error(e);
		}
	}

	async function del(id: string) {
		await api.deleteScene(id);
		if (editing?.id === id) editing = null;
		await refresh();
	}

	async function addLayer() {
		if (!editing || !addSourceId) return;
		try {
			// A media-library asset (image/video) becomes a source first, then a layer.
			let sourceId = addSourceId;
			if (sourceId.startsWith('asset:')) {
				const src = await api.createSourceFromAsset(sourceId.slice(6));
				sourceId = src.id;
				sources.set(await api.listSources());
			}
			const n = editing.layers.length;
			const layer =
				n === 0
					? { source_id: sourceId, x: 0, y: 0, width: outW, height: outH, z_index: 0, opacity: 1 }
					: { source_id: sourceId, x: Math.round(outW * 0.6), y: Math.round(outH * 0.57), width: Math.round(outW / 3), height: Math.round(outH / 3), z_index: n, opacity: 1 };
			await api.addLayer(editing.id, layer);
			addSourceId = '';
			await refresh();
		} catch (e) {
			notify.error(e);
		}
	}

	async function removeLayer(id: string) {
		if (!editing) return;
		await api.deleteLayer(editing.id, id);
		if (selectedLayer === id) selectedLayer = null;
		await refresh();
	}

	async function saveLayer(l: Layer) {
		if (!editing) return;
		try {
			await api.updateLayer(editing.id, l.id, {
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

	// --- drag + resize on the canvas ---
	let drag: {
		id: string;
		mode: 'move' | 'resize';
		sx: number;
		sy: number;
		ox: number;
		oy: number;
		ow: number;
		oh: number;
	} | null = null;

	function onDown(l: Layer, mode: 'move' | 'resize', e: PointerEvent) {
		e.preventDefault();
		e.stopPropagation();
		selectedLayer = l.id;
		drag = { id: l.id, mode, sx: e.clientX, sy: e.clientY, ox: l.position.x, oy: l.position.y, ow: l.size.width, oh: l.size.height };
		window.addEventListener('pointermove', onMove);
		window.addEventListener('pointerup', onUp);
	}

	function onMove(e: PointerEvent) {
		if (!drag || !editing || !canvasEl) return;
		const rect = canvasEl.getBoundingClientRect();
		const dx = ((e.clientX - drag.sx) / rect.width) * outW;
		const dy = ((e.clientY - drag.sy) / rect.height) * outH;
		const l = editing.layers.find((x) => x.id === drag!.id);
		if (!l) return;
		if (drag.mode === 'move') {
			l.position.x = Math.max(0, Math.min(outW - l.size.width, drag.ox + dx));
			l.position.y = Math.max(0, Math.min(outH - l.size.height, drag.oy + dy));
		} else {
			l.size.width = Math.max(80, Math.min(outW - l.position.x, drag.ow + dx));
			l.size.height = Math.max(45, Math.min(outH - l.position.y, drag.oh + dy));
		}
	}

	async function onUp() {
		window.removeEventListener('pointermove', onMove);
		window.removeEventListener('pointerup', onUp);
		const d = drag;
		drag = null;
		if (d && editing) {
			const l = editing.layers.find((x) => x.id === d.id);
			if (l) await saveLayer(l);
		}
	}

	async function nudgeZ(l: Layer, dir: 1 | -1) {
		l.z_index = Math.max(0, l.z_index + dir);
		await saveLayer(l);
		await refresh();
	}
	async function setOpacity(l: Layer, v: number) {
		l.opacity = v;
		await saveLayer(l);
	}
	async function toggleVisible(l: Layer) {
		l.visible = !l.visible;
		await saveLayer(l);
		await refresh();
	}
</script>

<div class="mx-auto flex max-w-6xl gap-4">
	<!-- Scene list -->
	<aside class="w-56 shrink-0 space-y-3">
		<section class="panel">
			<header class="panel__head">▮ Scenes</header>
			<div class="panel__body space-y-2">
				<form onsubmit={(e) => { e.preventDefault(); create(); }} class="space-y-2">
					<div>
						<label class="field-label" for="scene-name">Scene name</label>
						<input id="scene-name" bind:value={name} placeholder="e.g. Intro" class="input" />
					</div>
					<div>
						<label class="field-label" for="scene-preset">Layout preset</label>
						<select id="scene-preset" bind:value={preset} class="select w-full">
							<option value="blank">Blank scene</option>
							<option value="fullscreen">Full screen (1 source)</option>
							<option value="pip">Picture-in-picture (2 sources)</option>
							<option value="split">Split screen (2 sources)</option>
						</select>
					</div>
					{#if preset !== 'blank'}
						<div>
							<label class="field-label" for="preset-a">{preset === 'fullscreen' ? 'Source' : 'Main source'}</label>
							<select id="preset-a" bind:value={presetSourceA} class="select w-full">
								<option value="">Select source…</option>
								{#each $sources as s}<option value={s.id}>{s.name}</option>{/each}
								{#each assets as a}<option value="asset:{a.id}">🖼 {a.name}</option>{/each}
							</select>
						</div>
					{/if}
					{#if preset === 'pip' || preset === 'split'}
						<div>
							<label class="field-label" for="preset-b">{preset === 'pip' ? 'Corner source' : 'Second source'}</label>
							<select id="preset-b" bind:value={presetSourceB} class="select w-full">
								<option value="">Select source…</option>
								{#each $sources as s}<option value={s.id}>{s.name}</option>{/each}
								{#each assets as a}<option value="asset:{a.id}">🖼 {a.name}</option>{/each}
							</select>
						</div>
					{/if}
					<button type="submit" disabled={creating || !name.trim() || (preset !== 'blank' && !presetSourceA) || ((preset === 'pip' || preset === 'split') && !presetSourceB)} class="btn w-full">
						{creating ? 'Creating…' : '+ Create Scene'}
					</button>
				</form>
				<div class="border-t border-border-dim pt-2"></div>
				{#each $scenes as scene (scene.id)}
					<div class="flex items-center gap-1">
						<button
							onclick={() => { editing = scene; selectedLayer = null; }}
							class="rack-item flex-1 justify-between {editing?.id === scene.id ? 'rack-item--active' : ''}"
						>
							<span class="truncate">{scene.name}</span>
							<span class="pill {activeId === scene.id ? 'pill--live' : 'pill--idle'} text-[9px]">
								{activeId === scene.id ? '● LIVE' : scene.layers.length}
							</span>
						</button>
						<button onclick={() => duplicate(scene.id)} class="btn btn--ghost shrink-0 px-2" title="Duplicate scene" aria-label="Duplicate scene">⧉</button>
					</div>
				{/each}
			</div>
		</section>
	</aside>

	<!-- Editor -->
	<main class="min-w-0 flex-1 space-y-3">
		{#if !editing}
			<div class="panel"><div class="panel__body text-center text-sm text-amber-muted">Create or select a scene to start.</div></div>
		{:else}
			<section class="panel">
				<header class="panel__head">
					<span>▮ {editing.name}</span>
					<div class="flex gap-2">
						<button onclick={() => activate(editing!.id)} class="btn btn--go">● Take to Program</button>
						<button onclick={() => duplicate(editing!.id)} class="btn">⧉ Duplicate</button>
						<button onclick={() => del(editing!.id)} class="btn btn--danger">Delete</button>
					</div>
				</header>
				<div class="panel__body space-y-3">
					<!-- Canvas -->
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						bind:this={canvasEl}
						class="scanlines-well relative w-full overflow-hidden rounded-md border border-border bg-black"
						style="aspect-ratio: 16 / 9"
						onpointerdown={() => (selectedLayer = null)}
					>
						{#each orderedLayers as l (l.id)}
							<!-- svelte-ignore a11y_no_static_element_interactions -->
							<div
								class="layer {selectedLayer === l.id ? 'layer--sel' : ''} {l.visible === false ? 'layer--hidden' : ''}"
								style="left:{(l.position.x / outW) * 100}%;top:{(l.position.y / outH) * 100}%;width:{(l.size.width / outW) * 100}%;height:{(l.size.height / outH) * 100}%;opacity:{l.visible === false ? 0.35 : l.opacity}"
								onpointerdown={(e) => onDown(l, 'move', e)}
							>
								<span class="layer__label">{srcName(l.source_id)}{l.visible === false ? ' (hidden)' : ''}</span>
								<!-- svelte-ignore a11y_no_static_element_interactions -->
								<div class="layer__resize" onpointerdown={(e) => onDown(l, 'resize', e)}></div>
							</div>
						{/each}
						{#if editing.layers.length === 0}
							<div class="absolute inset-0 flex items-center justify-center text-sm text-amber-muted">
								Add a source layer below
							</div>
						{/if}
					</div>

					<!-- Add layer -->
					<div class="flex items-end gap-2">
						<div class="flex-1">
							<label class="field-label" for="add-src">Add layer (source or image)</label>
							<select id="add-src" bind:value={addSourceId} class="select">
								<option value="">Select source or image…</option>
								{#if $sources.length > 0}
									<optgroup label="Sources">
										{#each $sources as s}
											<option value={s.id}>{s.name}</option>
										{/each}
									</optgroup>
								{/if}
								{#if assets.length > 0}
									<optgroup label="Media Library">
										{#each assets as a}
											<option value="asset:{a.id}">{a.name}</option>
										{/each}
									</optgroup>
								{/if}
							</select>
						</div>
						<button onclick={addLayer} disabled={!addSourceId} class="btn">+ Add Layer</button>
					</div>

					<!-- Selected layer: exact geometry -->
					{#if selected}
						<div class="rounded-md border border-border-dim p-3">
							<p class="mb-2 text-[11px] uppercase tracking-wide text-amber-muted">
								Selected: {srcName(selected.source_id)}
							</p>
							<div class="grid grid-cols-4 gap-2">
								<div>
									<label class="field-label" for="geo-x">X</label>
									<input id="geo-x" type="number" min="0" max={outW} bind:value={selected.position.x} onchange={() => saveLayer(selected!)} class="input" />
								</div>
								<div>
									<label class="field-label" for="geo-y">Y</label>
									<input id="geo-y" type="number" min="0" max={outH} bind:value={selected.position.y} onchange={() => saveLayer(selected!)} class="input" />
								</div>
								<div>
									<label class="field-label" for="geo-w">Width</label>
									<input id="geo-w" type="number" min="80" max={outW} bind:value={selected.size.width} onchange={() => saveLayer(selected!)} class="input" />
								</div>
								<div>
									<label class="field-label" for="geo-h">Height</label>
									<input id="geo-h" type="number" min="45" max={outH} bind:value={selected.size.height} onchange={() => saveLayer(selected!)} class="input" />
								</div>
							</div>
						</div>
					{/if}
				</div>
			</section>

			<!-- Layer list / controls -->
			<section class="panel">
				<header class="panel__head">▮ Layers (top = front)</header>
				<div class="panel__body space-y-1">
					{#if editing.layers.length === 0}
						<p class="text-xs text-amber-muted">No layers yet.</p>
					{:else}
						{#each [...orderedLayers].reverse() as l (l.id)}
							<div class="row items-center {selectedLayer === l.id ? 'border-[--color-amber]' : ''}">
								<button class="flex-1 truncate text-left text-amber-dim" onclick={() => (selectedLayer = l.id)}>
									<span class="text-amber-muted">▸</span>
									{srcName(l.source_id)}
									<span class="text-[11px] text-amber-muted">
										· {Math.round(l.size.width)}×{Math.round(l.size.height)} @ {Math.round(l.position.x)},{Math.round(l.position.y)} · z{l.z_index}
									</span>
								</button>
								<div class="flex items-center gap-1">
									<button class="btn btn--ghost" onclick={() => toggleVisible(l)} title={l.visible === false ? 'Show layer' : 'Hide layer'} aria-label={l.visible === false ? 'Show layer' : 'Hide layer'}>{l.visible === false ? '👁‍🗨' : '👁'}</button>
									<select
										bind:value={l.fit}
										onchange={() => saveLayer(l)}
										class="select h-7 py-0 text-[11px]" title="Fit mode"
										aria-label="Fit mode"
									>
										<option value="fill">Fill</option>
										<option value="contain">Contain</option>
										<option value="cover">Cover</option>
									</select>
									<input
										type="range" min="0" max="1" step="0.05" value={l.opacity}
										oninput={(e) => setOpacity(l, +e.currentTarget.value)}
										class="w-20" title="Opacity"
									/>
									<button class="btn btn--ghost" onclick={() => nudgeZ(l, 1)} title="Bring forward" aria-label="Bring forward">▲</button>
									<button class="btn btn--ghost" onclick={() => nudgeZ(l, -1)} title="Send back" aria-label="Send back">▼</button>
									<button class="btn btn--danger" onclick={() => removeLayer(l.id)}>✕</button>
								</div>
							</div>
						{/each}
					{/if}
				</div>
			</section>
		{/if}
	</main>
</div>

<style>
	.layer {
		position: absolute;
		border: 1px solid var(--color-amber-dim);
		background: rgba(255, 176, 0, 0.06);
		cursor: move;
		touch-action: none;
		box-sizing: border-box;
	}
	.layer--sel {
		border-color: var(--color-amber-bright);
		box-shadow: 0 0 0 1px var(--color-amber-bright);
		background: rgba(255, 176, 0, 0.12);
	}
	.layer--hidden {
		border-style: dashed;
		border-color: var(--color-amber-muted);
		background: rgba(128, 128, 128, 0.08);
	}
	.layer__label {
		position: absolute;
		top: 2px;
		left: 4px;
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--color-amber-bright);
		pointer-events: none;
		text-shadow: 0 1px 2px #000;
	}
	.layer__resize {
		position: absolute;
		right: -5px;
		bottom: -5px;
		width: 12px;
		height: 12px;
		background: var(--color-amber-bright);
		border: 1px solid var(--color-bg);
		border-radius: 2px;
		cursor: nwse-resize;
		touch-action: none;
	}
</style>
