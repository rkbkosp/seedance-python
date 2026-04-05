<script lang="ts">
  import { onMount } from "svelte";
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";

  import { fileToPayload } from "./lib/file-input";
  import { loadDraftPrompt, saveDraftPrompt } from "./lib/storage";
  import type {
    AppSettings,
    BootstrapPayload,
    CreateGenerationRequest,
    FileInputPayload,
    GenerationDetail,
    GenerationSummary,
    GenerationUpdatedEvent,
    HistoryPage,
  } from "./lib/types";

  const DEFAULT_SETTINGS: AppSettings = {
    apiKey: "",
    platform: "volc",
    model: "",
    baseUrl: "",
    pollInterval: 3,
  };

  const ratioOptions = ["16:9", "4:3", "1:1", "3:4", "9:16", "21:9", "adaptive"];
  const resolutionOptions = ["480p", "720p", "1080p"];

  let settings: AppSettings = { ...DEFAULT_SETTINGS };
  let form = {
    prompt: loadDraftPrompt(),
    ratio: "16:9",
    resolution: "720p",
    duration: 5,
    frames: "",
    returnLastFrame: true,
    draft: false,
    cameraFixed: false,
    watermark: false,
    generateAudio: false,
    seed: "",
  };

  let firstFrame: FileInputPayload | null = null;
  let inputLastFrame: FileInputPayload | null = null;
  let referenceImages: FileInputPayload[] = [];

  let history: HistoryPage = { items: [], page: 1, pageSize: 10, total: 0 };
  let activeTasks: GenerationSummary[] = [];
  let selectedGeneration: GenerationDetail | null = null;

  let dataDir = "";
  let artifactsDir = "";
  let statusFilter = "";
  let previewId: number | null = null;
  let previewTimer: number | null = null;

  let isBootstrapping = true;
  let isSavingSettings = false;
  let isSubmitting = false;
  let drawerOpen = false;
  let feedback = "";
  let errorMessage = "";

  const formatDate = new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });

  function assetSrc(path?: string | null): string | null {
    return path ? convertFileSrc(path) : null;
  }

  function statusLabel(status: string): string {
    return status.replaceAll("_", " ");
  }

  function displayDate(value?: string | null): string {
    if (!value) return "--";
    return formatDate.format(new Date(value));
  }

  function revokePreview(asset: FileInputPayload | null): void {
    if (asset?.previewUrl?.startsWith("blob:")) {
      URL.revokeObjectURL(asset.previewUrl);
    }
  }

  function setFeedback(message: string): void {
    feedback = message;
    errorMessage = "";
  }

  function setError(message: string): void {
    errorMessage = message;
    feedback = "";
  }

  async function bootstrap(): Promise<void> {
    isBootstrapping = true;
    try {
      const payload = await invoke<BootstrapPayload>("bootstrap");
      settings = {
        ...DEFAULT_SETTINGS,
        ...payload.settings,
        model: payload.settings.model ?? "",
        baseUrl: payload.settings.baseUrl ?? "",
      };
      activeTasks = payload.activeTasks;
      history = payload.history;
      dataDir = payload.dataDir;
      artifactsDir = payload.artifactsDir;
      setFeedback("Studio ready");
    } catch (error) {
      setError(String(error));
    } finally {
      isBootstrapping = false;
    }
  }

  async function refreshHistory(page = history.page): Promise<void> {
    history = await invoke<HistoryPage>("list_generations", {
      page,
      pageSize: history.pageSize || 10,
      status: statusFilter || null,
    });
  }

  async function refreshActiveTasks(): Promise<void> {
    activeTasks = await invoke<GenerationSummary[]>("list_active_generations");
  }

  async function refreshSelectedGeneration(): Promise<void> {
    if (!selectedGeneration) return;
    selectedGeneration = await invoke<GenerationDetail>("get_generation", {
      generationId: selectedGeneration.id,
    });
  }

  async function saveSettingsAction(): Promise<void> {
    isSavingSettings = true;
    try {
      settings = await invoke<AppSettings>("save_settings", { settings });
      settings = {
        ...settings,
        model: settings.model ?? "",
        baseUrl: settings.baseUrl ?? "",
      };
      setFeedback("Settings saved");
    } catch (error) {
      setError(String(error));
    } finally {
      isSavingSettings = false;
    }
  }

  async function submitGeneration(): Promise<void> {
    isSubmitting = true;
    try {
      const request: CreateGenerationRequest = {
        prompt: form.prompt,
        firstFrame,
        lastFrame: inputLastFrame,
        referenceImages,
        ratio: form.ratio || null,
        resolution: form.resolution || null,
        duration: form.frames ? null : form.duration || null,
        frames: form.frames ? Number(form.frames) : null,
        returnLastFrame: form.returnLastFrame,
        draft: form.draft,
        cameraFixed: form.cameraFixed,
        watermark: form.watermark,
        generateAudio: form.generateAudio,
        seed: form.seed ? Number(form.seed) : null,
      };

      const created = await invoke<GenerationDetail>("create_generation", { request });
      selectedGeneration = created;
      drawerOpen = true;
      await Promise.all([refreshActiveTasks(), refreshHistory(1)]);
      setFeedback(`Queued generation #${created.id}`);
    } catch (error) {
      setError(String(error));
    } finally {
      isSubmitting = false;
    }
  }

  async function openDetail(generationId: number): Promise<void> {
    try {
      selectedGeneration = await invoke<GenerationDetail>("get_generation", {
        generationId,
      });
      drawerOpen = true;
    } catch (error) {
      setError(String(error));
    }
  }

  async function openInSystem(path: string): Promise<void> {
    try {
      await invoke("open_in_file_manager", { path });
    } catch (error) {
      setError(String(error));
    }
  }

  async function handleSingleFileChange(
    event: Event,
    kind: "first" | "last",
  ): Promise<void> {
    const target = event.currentTarget as HTMLInputElement | null;
    const file = target?.files?.[0];
    if (!file) return;

    const payload = await fileToPayload(file);
    if (kind === "first") {
      revokePreview(firstFrame);
      firstFrame = payload;
    } else {
      revokePreview(inputLastFrame);
      inputLastFrame = payload;
    }
    if (target) target.value = "";
  }

  async function handleReferenceFilesChange(event: Event): Promise<void> {
    const target = event.currentTarget as HTMLInputElement | null;
    const files = Array.from(target?.files ?? []);
    if (!files.length) return;

    const payloads = await Promise.all(files.map(fileToPayload));
    referenceImages = [...referenceImages, ...payloads];
    if (target) target.value = "";
  }

  function removeReference(index: number): void {
    const [removed] = referenceImages.splice(index, 1);
    revokePreview(removed);
    referenceImages = [...referenceImages];
  }

  function replaceReferenceImages(nextAssets: FileInputPayload[]): void {
    referenceImages.forEach((asset) => revokePreview(asset));
    referenceImages = nextAssets;
  }

  function usePrompt(prompt: string): void {
    form.prompt = prompt;
    saveDraftPrompt(prompt);
    setFeedback("Prompt loaded into composer");
  }

  function storedAsset(path: string | null | undefined): FileInputPayload | null {
    if (!path) return null;
    return {
      existingPath: path,
      fileName: path.split(/[\\/]/).pop() ?? "asset",
      previewUrl: assetSrc(path),
    };
  }

  function reuseAssets(detail: GenerationDetail): void {
    revokePreview(firstFrame);
    revokePreview(inputLastFrame);
    firstFrame = storedAsset(detail.firstFramePath);
    inputLastFrame = storedAsset(detail.inputLastFramePath);
    replaceReferenceImages(detail.referenceImages.map((path) => ({
      existingPath: path,
      fileName: path.split(/[\\/]/).pop() ?? "reference",
      previewUrl: assetSrc(path),
    })));
    setFeedback("Visual references loaded into composer");
  }

  function applyGenerationParams(detail: GenerationDetail): void {
    try {
      const params = JSON.parse(detail.paramsJson) as Record<string, unknown>;
      form = {
        ...form,
        ratio: typeof params.ratio === "string" ? params.ratio : form.ratio,
        resolution: typeof params.resolution === "string" ? params.resolution : form.resolution,
        duration: typeof params.duration === "number" ? params.duration : form.duration,
        frames: typeof params.frames === "number" ? String(params.frames) : "",
        returnLastFrame:
          typeof params.returnLastFrame === "boolean" ? params.returnLastFrame : form.returnLastFrame,
        draft: typeof params.draft === "boolean" ? params.draft : form.draft,
        cameraFixed: typeof params.cameraFixed === "boolean" ? params.cameraFixed : form.cameraFixed,
        watermark: typeof params.watermark === "boolean" ? params.watermark : form.watermark,
        generateAudio:
          typeof params.generateAudio === "boolean" ? params.generateAudio : form.generateAudio,
        seed: typeof params.seed === "number" ? String(params.seed) : "",
      };
    } catch {
      // Keep the current composer settings if old records stored invalid JSON.
    }
  }

  function applyGenerationToComposer(detail: GenerationDetail): void {
    usePrompt(detail.prompt);
    applyGenerationParams(detail);
    reuseAssets(detail);
    setFeedback(`Generation #${detail.id} loaded into the composer`);
  }

  function updatePromptDraft(value: string): void {
    form.prompt = value;
    saveDraftPrompt(value);
  }

  function startPromptDrag(event: DragEvent, prompt: string): void {
    if (!event.dataTransfer) return;
    const payload = JSON.stringify({ kind: "prompt", prompt });
    event.dataTransfer.effectAllowed = "copy";
    event.dataTransfer.setData("text/plain", prompt);
    event.dataTransfer.setData("application/x-seedance-item", payload);
  }

  function startAssetDrag(event: DragEvent, path: string, slot: string): void {
    if (!event.dataTransfer) return;
    event.dataTransfer.effectAllowed = "copy";
    event.dataTransfer.setData(
      "application/x-seedance-item",
      JSON.stringify({ kind: "asset", path, slot }),
    );
    event.dataTransfer.setData("text/plain", path);
  }

  function fileNameFromPath(path: string): string {
    return path.split(/[\\/]/).pop() ?? "seedance-asset";
  }

  function mimeFromPath(path: string): string {
    const lower = path.toLowerCase();
    if (lower.endsWith(".mp4")) return "video/mp4";
    if (lower.endsWith(".mov")) return "video/quicktime";
    if (lower.endsWith(".webm")) return "video/webm";
    if (lower.endsWith(".m4v")) return "video/x-m4v";
    return "application/octet-stream";
  }

  function toFileUri(path: string): string {
    const normalized = path.replace(/\\/g, "/");
    const withLeadingSlash = /^[a-zA-Z]:\//.test(normalized)
      ? `/${normalized}`
      : normalized.startsWith("/")
        ? normalized
        : `/${normalized}`;
    return encodeURI(`file://${withLeadingSlash}`);
  }

  function startVideoExportDrag(event: DragEvent, path: string): void {
    if (!event.dataTransfer) return;
    const fileName = fileNameFromPath(path);
    const fileUri = toFileUri(path);
    const mime = mimeFromPath(path);
    event.dataTransfer.effectAllowed = "copyLink";
    event.dataTransfer.setData("text/plain", path);
    event.dataTransfer.setData("text/uri-list", fileUri);
    event.dataTransfer.setData("DownloadURL", `${mime}:${fileName}:${fileUri}`);
  }

  function parseCustomDrop(event: DragEvent): { kind: string; prompt?: string; path?: string } | null {
    const raw = event.dataTransfer?.getData("application/x-seedance-item");
    if (!raw) return null;
    try {
      return JSON.parse(raw);
    } catch {
      return null;
    }
  }

  function handlePromptDrop(event: DragEvent): void {
    event.preventDefault();
    const payload = parseCustomDrop(event);
    if (payload?.kind === "prompt" && payload.prompt) {
      updatePromptDraft(payload.prompt);
      setFeedback("Prompt dropped into composer");
    }
  }

  function handleAssetDrop(event: DragEvent, slot: "first" | "last" | "reference"): void {
    event.preventDefault();
    const payload = parseCustomDrop(event);
    if (payload?.kind !== "asset" || !payload.path) return;
    const asset = storedAsset(payload.path);
    if (!asset) return;

    if (slot === "first") {
      revokePreview(firstFrame);
      firstFrame = asset;
      return;
    }
    if (slot === "last") {
      revokePreview(inputLastFrame);
      inputLastFrame = asset;
      return;
    }
    referenceImages = [...referenceImages, asset];
  }

  function allowDrop(event: DragEvent): void {
    event.preventDefault();
  }

  function armPreview(id: number): void {
    clearPreviewTimer();
    previewTimer = window.setTimeout(() => {
      previewId = id;
    }, 150);
  }

  function clearPreview(): void {
    clearPreviewTimer();
    previewId = null;
  }

  function clearPreviewTimer(): void {
    if (previewTimer) {
      window.clearTimeout(previewTimer);
      previewTimer = null;
    }
  }

  function totalPages(): number {
    return Math.max(1, Math.ceil(history.total / Math.max(1, history.pageSize)));
  }

  async function changePage(nextPage: number): Promise<void> {
    if (nextPage < 1 || nextPage > totalPages()) return;
    try {
      await refreshHistory(nextPage);
    } catch (error) {
      setError(String(error));
    }
  }

  async function changeFilter(nextFilter: string): Promise<void> {
    statusFilter = nextFilter;
    try {
      await refreshHistory(1);
    } catch (error) {
      setError(String(error));
    }
  }

  onMount(() => {
    let unlisten = () => {};

    void (async () => {
      await bootstrap();
      unlisten = await listen<GenerationUpdatedEvent>(
        "generation-updated",
        async ({ payload }) => {
          try {
            await Promise.all([refreshActiveTasks(), refreshHistory(history.page)]);
            if (selectedGeneration && payload.generationId === selectedGeneration.id) {
              await refreshSelectedGeneration();
            }
          } catch (error) {
            setError(String(error));
          }
        },
      );
    })();

    return () => {
      unlisten();
      revokePreview(firstFrame);
      revokePreview(inputLastFrame);
      referenceImages.forEach((asset) => revokePreview(asset));
    };
  });

  function openSummaryVideo(item: GenerationSummary): void {
    if (item.videoPath) {
      void openInSystem(item.videoPath);
    }
  }

  function startSummaryVideoExportDrag(event: DragEvent, item: GenerationSummary): void {
    if (item.videoPath) {
      startVideoExportDrag(event, item.videoPath);
    }
  }

  async function reuseGenerationById(generationId: number): Promise<void> {
    try {
      const detail = await invoke<GenerationDetail>("get_generation", { generationId });
      selectedGeneration = detail;
      applyGenerationToComposer(detail);
    } catch (error) {
      setError(String(error));
    }
  }

  function loadSelectedPrompt(): void {
    if (selectedGeneration) {
      usePrompt(selectedGeneration.prompt);
    }
  }

  function loadSelectedAssets(): void {
    if (selectedGeneration) {
      reuseAssets(selectedGeneration);
    }
  }

  function reuseSelectedGeneration(): void {
    if (selectedGeneration) {
      applyGenerationToComposer(selectedGeneration);
    }
  }

  function openSelectedVideo(): void {
    if (selectedGeneration?.videoPath) {
      void openInSystem(selectedGeneration.videoPath);
    }
  }

  function startSelectedVideoExportDrag(event: DragEvent): void {
    if (selectedGeneration?.videoPath) {
      startVideoExportDrag(event, selectedGeneration.videoPath);
    }
  }

  function startSelectedPromptDrag(event: DragEvent): void {
    if (selectedGeneration) {
      startPromptDrag(event, selectedGeneration.prompt);
    }
  }

  function startSelectedAssetDrag(event: DragEvent, path: string | null | undefined, slot: string): void {
    if (path) {
      startAssetDrag(event, path, slot);
    }
  }

  function loadSelectedAssetToSlot(slot: "first" | "last" | "reference", path: string | null | undefined): void {
    loadAssetIntoSlot(slot, path);
  }

  function loadAssetIntoSlot(slot: "first" | "last" | "reference", path: string | null | undefined): void {
    const asset = storedAsset(path);
    if (!asset) return;

    if (slot === "first") {
      revokePreview(firstFrame);
      firstFrame = asset;
      setFeedback("First frame loaded from history");
      return;
    }

    if (slot === "last") {
      revokePreview(inputLastFrame);
      inputLastFrame = asset;
      setFeedback("Last frame loaded from history");
      return;
    }

    referenceImages = [...referenceImages, asset];
    setFeedback("Reference image appended from history");
  }
</script>

<svelte:head>
  <title>Seedance Studio</title>
</svelte:head>

<div class="studio-shell">
  <header class="hero">
    <div>
      <p class="eyebrow">Seedance Studio</p>
      <h1>Prompt editing, history replay, and local media control in one desktop workspace.</h1>
      <p class="hero-copy">
        The app keeps every generation on disk, previews history with light thumbnails, and lets you pull old prompts
        and reference assets back into the composer without leaving the window.
      </p>
    </div>
    <div class="meta-panel">
      <div>
        <span class="meta-label">Data</span>
        <span class="meta-value">{dataDir || "Bootstrapping..."}</span>
      </div>
      <div>
        <span class="meta-label">Artifacts</span>
        <span class="meta-value">{artifactsDir || "Preparing storage..."}</span>
      </div>
      <div class:success={Boolean(feedback)} class:error={Boolean(errorMessage)}>
        {#if errorMessage}
          <span>{errorMessage}</span>
        {:else}
          <span>{feedback}</span>
        {/if}
      </div>
    </div>
  </header>

  <section class="top-grid">
    <div class="panel composer-panel">
      <div class="panel-header">
        <div>
          <p class="panel-kicker">Composer</p>
          <h2>Build a new generation</h2>
        </div>
        <button class="primary-button" disabled={isSubmitting || isBootstrapping} on:click={submitGeneration}>
          {#if isSubmitting}Queueing...{:else}Generate{/if}
        </button>
      </div>

      <label class="field">
        <span>Prompt</span>
        <textarea
          class="prompt-field"
          placeholder="Describe movement, camera motion, atmosphere, and timing."
          value={form.prompt}
          on:input={(event) => updatePromptDraft((event.currentTarget as HTMLTextAreaElement).value)}
          on:dragover={allowDrop}
          on:drop={handlePromptDrop}
        ></textarea>
      </label>

      <div class="asset-grid">
        <div
          class="field drop-field"
          role="group"
          aria-label="First frame drop zone"
          on:dragover={allowDrop}
          on:drop={(event) => handleAssetDrop(event, "first")}
        >
          <div class="field-head">
            <span>First frame</span>
            {#if firstFrame}
              <button class="link-button" on:click={() => { revokePreview(firstFrame); firstFrame = null; }}>
                Clear
              </button>
            {/if}
          </div>
          <label class="asset-box">
            {#if firstFrame?.previewUrl}
              <img src={firstFrame.previewUrl} alt="First frame preview" />
            {:else}
              <span>Drop a reusable image here or upload a new one.</span>
            {/if}
            <input accept="image/*" type="file" on:change={(event) => handleSingleFileChange(event, "first")} />
          </label>
        </div>

        <div
          class="field drop-field"
          role="group"
          aria-label="Input last frame drop zone"
          on:dragover={allowDrop}
          on:drop={(event) => handleAssetDrop(event, "last")}
        >
          <div class="field-head">
            <span>Input last frame</span>
            {#if inputLastFrame}
              <button class="link-button" on:click={() => { revokePreview(inputLastFrame); inputLastFrame = null; }}>
                Clear
              </button>
            {/if}
          </div>
          <label class="asset-box">
            {#if inputLastFrame?.previewUrl}
              <img src={inputLastFrame.previewUrl} alt="Input last frame preview" />
            {:else}
              <span>Set the final composition target before generation starts.</span>
            {/if}
            <input accept="image/*" type="file" on:change={(event) => handleSingleFileChange(event, "last")} />
          </label>
        </div>

        <div
          class="field drop-field wide"
          role="group"
          aria-label="Reference image drop zone"
          on:dragover={allowDrop}
          on:drop={(event) => handleAssetDrop(event, "reference")}
        >
          <div class="field-head">
            <span>Reference images</span>
            <label class="upload-chip">
              Add
              <input accept="image/*" multiple type="file" on:change={handleReferenceFilesChange} />
            </label>
          </div>
          <div class="reference-strip">
            {#if referenceImages.length}
              {#each referenceImages as asset, index}
                <div class="reference-card">
                  {#if asset.previewUrl}
                    <img src={asset.previewUrl} alt={`Reference ${index + 1}`} />
                  {/if}
                  <button class="remove-button" on:click={() => removeReference(index)}>Remove</button>
                </div>
              {/each}
            {:else}
              <p class="reference-placeholder">History assets dropped here will append without replacing the existing set.</p>
            {/if}
          </div>
        </div>
      </div>

      <div class="controls-grid">
        <label class="field">
          <span>Ratio</span>
          <select bind:value={form.ratio}>
            {#each ratioOptions as option}
              <option value={option}>{option}</option>
            {/each}
          </select>
        </label>

        <label class="field">
          <span>Resolution</span>
          <select bind:value={form.resolution}>
            {#each resolutionOptions as option}
              <option value={option}>{option}</option>
            {/each}
          </select>
        </label>

        <label class="field">
          <span>Duration (sec)</span>
          <input bind:value={form.duration} min="1" step="1" type="number" />
        </label>

        <label class="field">
          <span>Frames</span>
          <input bind:value={form.frames} min="1" step="1" placeholder="Optional" type="number" />
        </label>

        <label class="field">
          <span>Seed</span>
          <input bind:value={form.seed} min="0" step="1" placeholder="Optional" type="number" />
        </label>
      </div>

      <div class="toggle-row">
        <label><input bind:checked={form.returnLastFrame} type="checkbox" /> Return last frame</label>
        <label><input bind:checked={form.cameraFixed} type="checkbox" /> Camera fixed</label>
        <label><input bind:checked={form.watermark} type="checkbox" /> Watermark</label>
        <label><input bind:checked={form.generateAudio} type="checkbox" /> Generate audio</label>
        <label><input bind:checked={form.draft} type="checkbox" /> Draft mode</label>
      </div>
    </div>

    <aside class="panel settings-panel">
      <div class="panel-header">
        <div>
          <p class="panel-kicker">Connection</p>
          <h2>Saved settings</h2>
        </div>
        <button class="secondary-button" disabled={isSavingSettings} on:click={saveSettingsAction}>
          {#if isSavingSettings}Saving...{:else}Save{/if}
        </button>
      </div>

      <label class="field">
        <span>API key</span>
        <input bind:value={settings.apiKey} placeholder="ARK API key" type="password" />
      </label>

      <div class="controls-grid two-column">
        <label class="field">
          <span>Platform</span>
          <select bind:value={settings.platform}>
            <option value="volc">Volc</option>
            <option value="byteplus">BytePlus</option>
          </select>
        </label>

        <label class="field">
          <span>Poll interval</span>
          <input bind:value={settings.pollInterval} min="1" step="0.5" type="number" />
        </label>
      </div>

      <label class="field">
        <span>Model override</span>
        <input bind:value={settings.model} placeholder="Optional" type="text" />
      </label>

      <label class="field">
        <span>Base URL override</span>
        <input bind:value={settings.baseUrl} placeholder="Optional" type="text" />
      </label>

      <div class="settings-note">
        Keep these saved locally so the composer only needs prompt and image changes between runs.
      </div>
    </aside>
  </section>

  <section class="panel active-panel">
    <div class="panel-header compact">
      <div>
        <p class="panel-kicker">Live queue</p>
        <h2>In progress</h2>
      </div>
      <span class="count-pill">{activeTasks.length}</span>
    </div>

    {#if activeTasks.length}
      <div class="active-list">
        {#each activeTasks as item}
          <button class="active-card" on:click={() => openDetail(item.id)}>
            <div class="spinner-cluster">
              <span class="spinner-ring"></span>
              <span class={`status-chip status-${item.status}`}>{statusLabel(item.status)}</span>
            </div>
            <div class="active-copy">
              <strong>{item.promptSummary}</strong>
              <span>{item.progressText ?? "Waiting for remote progress..."}</span>
            </div>
            <time>{displayDate(item.updatedAt)}</time>
          </button>
        {/each}
      </div>
    {:else}
      <div class="empty-state small">No active generations right now.</div>
    {/if}
  </section>

  <section class="panel history-panel">
    <div class="panel-header">
      <div>
        <p class="panel-kicker">Archive</p>
        <h2>Generation history</h2>
      </div>
      <div class="history-toolbar">
        <select bind:value={statusFilter} on:change={(event) => changeFilter((event.currentTarget as HTMLSelectElement).value)}>
          <option value="">All statuses</option>
          <option value="queued">Queued</option>
          <option value="running">Running</option>
          <option value="succeeded">Succeeded</option>
          <option value="failed">Failed</option>
          <option value="cancelled">Cancelled</option>
          <option value="expired">Expired</option>
        </select>
        <div class="pagination">
          <button class="secondary-button" disabled={history.page <= 1} on:click={() => changePage(history.page - 1)}>
            Prev
          </button>
          <span>Page {history.page} / {totalPages()}</span>
          <button class="secondary-button" disabled={history.page >= totalPages()} on:click={() => changePage(history.page + 1)}>
            Next
          </button>
        </div>
      </div>
    </div>

    {#if history.items.length}
      <div class="history-grid">
        {#each history.items as item}
          <article class="history-card">
            <button
              class="history-media"
              draggable={Boolean(item.videoPath)}
              on:mouseenter={() => armPreview(item.id)}
              on:mouseleave={clearPreview}
              on:focus={() => armPreview(item.id)}
              on:blur={clearPreview}
              on:dragstart={(event) => startSummaryVideoExportDrag(event, item)}
              on:click={() => openDetail(item.id)}
            >
              <span class={`status-chip floating status-${item.status}`}>{statusLabel(item.status)}</span>
              {#if previewId === item.id && item.videoPath}
                <video autoplay loop muted playsinline preload="metadata" src={assetSrc(item.videoPath) ?? undefined}></video>
              {:else if item.thumbnailPath}
                <img alt={item.promptSummary} src={assetSrc(item.thumbnailPath) ?? undefined} />
              {:else}
                <div class="media-fallback">No thumbnail yet</div>
              {/if}
            </button>

            <div class="history-body">
              <div class="history-meta">
                <time>{displayDate(item.createdAt)}</time>
                <span>{item.referenceCount} refs</span>
              </div>
              <button
                class="history-prompt"
                draggable="true"
                on:click={() => usePrompt(item.prompt)}
                on:dragstart={(event) => startPromptDrag(event, item.prompt)}
              >
                {item.promptSummary}
              </button>
              <p class="history-progress">{item.progressText ?? item.errorMessage ?? "Ready for reuse"}</p>
            </div>

            <div class="history-actions">
              <button class="primary-button subtle" on:click={() => reuseGenerationById(item.id)}>Reuse all</button>
              <button class="secondary-button" on:click={() => usePrompt(item.prompt)}>Use prompt</button>
              <button class="secondary-button" on:click={() => openDetail(item.id)}>Details</button>
              {#if item.videoPath}
                <button class="secondary-button" on:click={() => openSummaryVideo(item)}>Show file</button>
                <button
                  class="secondary-button"
                  draggable="true"
                  on:dragstart={(event) => startSummaryVideoExportDrag(event, item)}
                >
                  Drag file out
                </button>
              {/if}
            </div>
          </article>
        {/each}
      </div>
    {:else}
      <div class="empty-state">No history for this filter yet.</div>
    {/if}
  </section>

  {#if drawerOpen && selectedGeneration}
    <aside class="drawer">
      <button
        type="button"
        class="drawer-backdrop"
        aria-label="Close detail panel"
        on:click={() => (drawerOpen = false)}
      ></button>
      <div class="drawer-panel">
        <div class="panel-header">
          <div>
            <p class="panel-kicker">Generation #{selectedGeneration.id}</p>
            <h2>{statusLabel(selectedGeneration.status)}</h2>
          </div>
          <button class="secondary-button" on:click={() => (drawerOpen = false)}>Close</button>
        </div>

        <div class="drawer-section">
          <div class="drawer-tools">
            <button class="primary-button subtle" on:click={reuseSelectedGeneration}>
              Reuse all
            </button>
            <button class="primary-button" on:click={loadSelectedPrompt}>Load prompt</button>
            <button class="secondary-button" on:click={loadSelectedAssets}>Load assets</button>
            {#if selectedGeneration.videoPath}
              <button class="secondary-button" on:click={openSelectedVideo}>
                Show video file
              </button>
              <button
                class="secondary-button"
                draggable="true"
                on:dragstart={startSelectedVideoExportDrag}
              >
                Drag video file out
              </button>
            {/if}
          </div>

          <div
            class="prompt-card"
            role="button"
            tabindex="0"
            aria-label="Load or drag prompt back into composer"
            draggable="true"
            on:click={loadSelectedPrompt}
            on:keydown={(event) => (event.key === "Enter" || event.key === " ") && loadSelectedPrompt()}
            on:dragstart={startSelectedPromptDrag}
          >
            <span class="prompt-label">Click to load or drag this prompt back into the composer.</span>
            <p>{selectedGeneration.prompt}</p>
          </div>
        </div>

        <div class="drawer-section">
          <div class="drawer-grid">
            <div class="media-stack large">
              <span>Rendered video</span>
              {#if selectedGeneration.videoPath}
                <!-- svelte-ignore a11y_media_has_caption -->
                <video controls playsinline preload="metadata" src={assetSrc(selectedGeneration.videoPath) ?? undefined}></video>
              {:else}
                <div class="media-fallback">Video not downloaded yet</div>
              {/if}
            </div>

            <div class="media-stack">
              <span>First frame</span>
              {#if selectedGeneration.firstFramePath}
                <button
                  class="asset-button"
                  draggable="true"
                  on:click={() => loadSelectedAssetToSlot("first", selectedGeneration?.firstFramePath)}
                  on:dragstart={(event) => startSelectedAssetDrag(event, selectedGeneration?.firstFramePath, "first")}
                >
                  <img alt="First frame" src={assetSrc(selectedGeneration.firstFramePath) ?? undefined} />
                </button>
              {:else}
                <div class="media-fallback">Not set</div>
              {/if}
            </div>

            <div class="media-stack">
              <span>Input last frame</span>
              {#if selectedGeneration.inputLastFramePath}
                <button
                  class="asset-button"
                  draggable="true"
                  on:click={() => loadSelectedAssetToSlot("last", selectedGeneration?.inputLastFramePath)}
                  on:dragstart={(event) =>
                    startSelectedAssetDrag(event, selectedGeneration?.inputLastFramePath, "last")}
                >
                  <img alt="Input last frame" src={assetSrc(selectedGeneration.inputLastFramePath) ?? undefined} />
                </button>
              {:else}
                <div class="media-fallback">Not set</div>
              {/if}
            </div>

            <div class="media-stack">
              <span>Returned last frame</span>
              {#if selectedGeneration.returnedLastFramePath}
                <button
                  class="asset-button"
                  draggable="true"
                  on:click={() => loadSelectedAssetToSlot("last", selectedGeneration?.returnedLastFramePath)}
                  on:dragstart={(event) =>
                    startSelectedAssetDrag(event, selectedGeneration?.returnedLastFramePath, "last")}
                >
                  <img
                    alt="Returned last frame"
                    src={assetSrc(selectedGeneration.returnedLastFramePath) ?? undefined}
                  />
                </button>
              {:else}
                <div class="media-fallback">Not returned</div>
              {/if}
            </div>
          </div>
        </div>

        <div class="drawer-section">
          <div class="drawer-grid references">
            {#each selectedGeneration.referenceImages as imagePath}
              <button
                class="asset-button"
                draggable="true"
                on:click={() => loadAssetIntoSlot("reference", imagePath)}
                on:dragstart={(event) => startAssetDrag(event, imagePath, "reference")}
              >
                <img alt="Reference asset" src={assetSrc(imagePath) ?? undefined} />
              </button>
            {/each}
          </div>
        </div>

        <div class="drawer-section info-grid">
          <div>
            <span class="meta-label">Task ID</span>
            <span class="meta-value">{selectedGeneration.taskId ?? "--"}</span>
          </div>
          <div>
            <span class="meta-label">Created</span>
            <span class="meta-value">{displayDate(selectedGeneration.createdAt)}</span>
          </div>
          <div>
            <span class="meta-label">Updated</span>
            <span class="meta-value">{displayDate(selectedGeneration.updatedAt)}</span>
          </div>
          <div>
            <span class="meta-label">Reference count</span>
            <span class="meta-value">{selectedGeneration.referenceCount}</span>
          </div>
        </div>

        <div class="drawer-section">
          <span class="meta-label">Parameters</span>
          <pre>{selectedGeneration.paramsJson}</pre>
        </div>
      </div>
    </aside>
  {/if}
</div>

<style>
  .studio-shell {
    display: grid;
    gap: 1.25rem;
    padding: 1.4rem;
  }

  .hero,
  .panel,
  .drawer-panel {
    border: 1px solid var(--line);
    background: var(--bg-card);
    box-shadow: var(--shadow);
    backdrop-filter: blur(18px);
  }

  .hero {
    display: grid;
    grid-template-columns: minmax(0, 2.2fr) minmax(320px, 0.9fr);
    gap: 1rem;
    border-radius: 28px;
    padding: 1.5rem;
  }

  .eyebrow,
  .panel-kicker {
    margin: 0 0 0.45rem;
    color: var(--accent);
    font-size: 0.8rem;
    font-weight: 700;
    letter-spacing: 0.16em;
    text-transform: uppercase;
  }

  h1,
  h2 {
    margin: 0;
    font-family: "Iowan Old Style", "Palatino Linotype", Georgia, serif;
    font-weight: 600;
    line-height: 1.05;
  }

  h1 {
    max-width: 13ch;
    font-size: clamp(2rem, 3.5vw, 3.55rem);
  }

  h2 {
    font-size: 1.45rem;
  }

  .hero-copy {
    max-width: 72ch;
    margin: 1rem 0 0;
    color: var(--text-dim);
    line-height: 1.6;
  }

  .meta-panel {
    display: grid;
    gap: 0.85rem;
    align-content: start;
  }

  .meta-panel > div {
    display: grid;
    gap: 0.25rem;
    padding: 0.95rem 1rem;
    border-radius: 20px;
    background: var(--bg-card-strong);
    border: 1px solid var(--line);
  }

  .meta-panel > div.success {
    border-color: rgba(109, 226, 187, 0.5);
  }

  .meta-panel > div.error {
    border-color: rgba(255, 142, 111, 0.5);
    color: #ffd4c8;
  }

  .meta-label {
    color: var(--text-dim);
    font-size: 0.74rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .meta-value {
    word-break: break-word;
  }

  .top-grid {
    display: grid;
    grid-template-columns: minmax(0, 2.1fr) minmax(320px, 0.95fr);
    gap: 1.25rem;
  }

  .panel {
    border-radius: 24px;
    padding: 1.25rem;
  }

  .panel-header {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    align-items: start;
    margin-bottom: 1rem;
  }

  .panel-header.compact {
    align-items: center;
  }

  .field {
    display: grid;
    gap: 0.45rem;
  }

  .field span {
    color: var(--text-dim);
    font-size: 0.84rem;
  }

  .prompt-field,
  input,
  select,
  pre {
    width: 100%;
    border: 1px solid var(--line);
    border-radius: 18px;
    background: rgba(7, 22, 18, 0.9);
    color: var(--text);
  }

  .prompt-field,
  input,
  select {
    padding: 0.85rem 0.95rem;
  }

  .prompt-field {
    min-height: 220px;
    resize: vertical;
    line-height: 1.6;
  }

  .asset-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 1rem;
    margin-top: 1rem;
  }

  .drop-field.wide {
    grid-column: 1 / -1;
  }

  .field-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.75rem;
  }

  .asset-box {
    position: relative;
    display: grid;
    place-items: center;
    min-height: 220px;
    border-radius: 20px;
    border: 1px dashed var(--line-strong);
    background: linear-gradient(180deg, rgba(10, 30, 25, 0.85), rgba(9, 20, 18, 0.98));
    overflow: hidden;
    text-align: center;
    color: var(--text-dim);
  }

  .asset-box img,
  .reference-card img,
  .media-stack img,
  .asset-button img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .asset-box input,
  .upload-chip input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }

  .reference-strip {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: 0.8rem;
  }

  .reference-card {
    position: relative;
    overflow: hidden;
    min-height: 120px;
    border-radius: 18px;
    border: 1px solid var(--line);
    background: rgba(10, 20, 18, 0.85);
  }

  .remove-button,
  .upload-chip,
  .link-button,
  .secondary-button,
  .primary-button {
    border: 0;
    border-radius: 999px;
    padding: 0.7rem 1rem;
    transition: transform 120ms ease, background 120ms ease;
  }

  .primary-button {
    background: linear-gradient(135deg, var(--accent), #96f2d0);
    color: #06221b;
    font-weight: 700;
  }

  .primary-button.subtle {
    background: linear-gradient(135deg, rgba(109, 226, 187, 0.24), rgba(150, 242, 208, 0.18));
    color: var(--text);
    border: 1px solid rgba(109, 226, 187, 0.26);
  }

  .secondary-button,
  .upload-chip,
  .link-button,
  .remove-button {
    background: rgba(110, 226, 188, 0.1);
    color: var(--text);
  }

  .primary-button:hover,
  .secondary-button:hover,
  .upload-chip:hover,
  .link-button:hover,
  .remove-button:hover {
    transform: translateY(-1px);
  }

  .controls-grid {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    gap: 0.9rem;
    margin-top: 1rem;
  }

  .controls-grid.two-column {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    margin-top: 0;
  }

  .toggle-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.9rem 1.2rem;
    margin-top: 1rem;
    color: var(--text-dim);
  }

  .toggle-row label {
    display: inline-flex;
    gap: 0.45rem;
    align-items: center;
  }

  .settings-note,
  .reference-placeholder,
  .history-progress,
  .empty-state {
    color: var(--text-dim);
  }

  .active-list {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 0.9rem;
  }

  .active-card {
    display: grid;
    gap: 0.85rem;
    text-align: left;
    border: 1px solid var(--line);
    border-radius: 20px;
    background: rgba(9, 27, 23, 0.85);
    padding: 1rem;
  }

  .spinner-cluster {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .spinner-ring {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2px solid rgba(109, 226, 187, 0.16);
    border-top-color: var(--accent);
    animation: spin 0.85s linear infinite;
  }

  .active-copy {
    display: grid;
    gap: 0.35rem;
  }

  .active-copy span {
    color: var(--text-dim);
  }

  .history-toolbar,
  .pagination,
  .history-meta,
  .history-actions,
  .drawer-tools {
    display: flex;
    gap: 0.75rem;
    align-items: center;
    flex-wrap: wrap;
  }

  .history-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 1rem;
  }

  .history-card {
    display: grid;
    gap: 0.9rem;
    padding: 0.85rem;
    border-radius: 22px;
    border: 1px solid var(--line);
    background: rgba(8, 23, 19, 0.84);
  }

  .history-media {
    position: relative;
    min-height: 210px;
    padding: 0;
    overflow: hidden;
    border: 0;
    border-radius: 18px;
    background: rgba(10, 20, 18, 0.75);
  }

  .history-media img,
  .history-media video,
  .media-stack video {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .floating {
    position: absolute;
    top: 0.8rem;
    left: 0.8rem;
    z-index: 1;
  }

  .status-chip {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0.34rem 0.72rem;
    border-radius: 999px;
    font-size: 0.74rem;
    font-weight: 700;
    text-transform: capitalize;
    border: 1px solid transparent;
  }

  .status-succeeded {
    background: rgba(109, 226, 187, 0.14);
    color: #baf6df;
    border-color: rgba(109, 226, 187, 0.3);
  }

  .status-failed,
  .status-cancelled,
  .status-expired {
    background: rgba(255, 142, 111, 0.14);
    color: #ffd0c3;
    border-color: rgba(255, 142, 111, 0.32);
  }

  .status-running,
  .status-queued {
    background: rgba(111, 171, 255, 0.14);
    color: #dbe7ff;
    border-color: rgba(111, 171, 255, 0.3);
  }

  .history-body {
    display: grid;
    gap: 0.55rem;
  }

  .history-meta,
  .history-progress {
    color: var(--text-dim);
    font-size: 0.86rem;
  }

  .history-prompt {
    margin: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    text-align: left;
    line-height: 1.45;
  }

  .count-pill {
    display: inline-flex;
    min-width: 2.2rem;
    justify-content: center;
    padding: 0.35rem 0.75rem;
    border-radius: 999px;
    background: rgba(109, 226, 187, 0.14);
    color: var(--accent);
  }

  .empty-state {
    display: grid;
    place-items: center;
    min-height: 160px;
    border: 1px dashed var(--line);
    border-radius: 20px;
  }

  .empty-state.small {
    min-height: 90px;
  }

  .drawer {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: flex;
    justify-content: flex-end;
  }

  .drawer-backdrop {
    position: absolute;
    inset: 0;
    border: 0;
    background: rgba(4, 10, 9, 0.58);
  }

  .drawer-panel {
    position: relative;
    width: min(920px, calc(100vw - 3rem));
    height: 100%;
    overflow: auto;
    border-left: 1px solid var(--line);
    background: rgba(7, 20, 17, 0.96);
    padding: 1.25rem;
  }

  .drawer-section {
    display: grid;
    gap: 0.9rem;
    margin-top: 1rem;
  }

  .prompt-card,
  pre {
    margin: 0;
    padding: 1rem;
    border-radius: 18px;
    background: rgba(10, 22, 20, 0.92);
    overflow: auto;
    line-height: 1.55;
  }

  .prompt-card {
    cursor: grab;
  }

  .prompt-label {
    color: var(--text-dim);
    font-size: 0.82rem;
  }

  .prompt-card p {
    margin: 0.65rem 0 0;
  }

  .drawer-grid {
    display: grid;
    grid-template-columns: minmax(0, 1.4fr) repeat(3, minmax(0, 0.9fr));
    gap: 0.85rem;
  }

  .drawer-grid.references {
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
  }

  .media-stack {
    display: grid;
    gap: 0.5rem;
  }

  .media-stack.large {
    grid-row: span 2;
  }

  .media-stack > span {
    color: var(--text-dim);
    font-size: 0.82rem;
  }

  .media-stack video,
  .media-stack img,
  .asset-button,
  .media-fallback {
    min-height: 150px;
    border-radius: 18px;
    border: 1px solid var(--line);
    background: rgba(10, 18, 17, 0.94);
  }

  .asset-button {
    padding: 0;
    overflow: hidden;
    cursor: pointer;
  }

  .media-fallback {
    display: grid;
    place-items: center;
    color: var(--text-dim);
  }

  .info-grid {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 0.8rem;
  }

  .info-grid > div {
    display: grid;
    gap: 0.25rem;
    padding: 0.85rem 0.9rem;
    border-radius: 18px;
    background: rgba(10, 20, 18, 0.8);
    border: 1px solid var(--line);
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (max-width: 1180px) {
    .hero,
    .top-grid {
      grid-template-columns: 1fr;
    }

    .controls-grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }

    .drawer-grid,
    .info-grid {
      grid-template-columns: 1fr 1fr;
    }
  }

  @media (max-width: 760px) {
    .studio-shell {
      padding: 0.9rem;
    }

    .asset-grid,
    .controls-grid,
    .drawer-grid,
    .info-grid {
      grid-template-columns: 1fr;
    }

    .drawer-panel {
      width: 100vw;
    }

    .history-toolbar {
      width: 100%;
      justify-content: space-between;
    }
  }
</style>
