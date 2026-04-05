<script lang="ts">
  import { onMount } from "svelte";
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";

  import { fileToPayload } from "./lib/file-input";
  import { loadDraftPrompt, saveDraftPrompt } from "./lib/storage";
  import type {
    AppSettings,
    BalanceSnapshot,
    BalanceUpdatedEvent,
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
    billingAccessKey: "",
    billingSecretKey: "",
    lowBalanceThreshold: 100,
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
  let balance: BalanceSnapshot = {};

  let dataDir = "";
  let artifactsDir = "";
  let statusFilter = "";
  let previewId: number | null = null;
  let previewTimer: number | null = null;

  let isBootstrapping = true;
  let isSavingSettings = false;
  let isSubmitting = false;
  let isRefreshingBalance = false;
  let isExportingSecretBundle = false;
  let isImportingSecretBundle = false;
  let settingsHydrated = false;
  let settingsSaveTimer: number | null = null;
  let lastSavedSettingsFingerprint = "";
  let drawerOpen = false;
  let feedback = "";
  let errorMessage = "";
  let secretBundlePassword = "";
  let secretBundleExport = "";
  let secretBundleImport = "";

  const formatDate = new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
  const amountFormatter = new Intl.NumberFormat(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
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

  function normalizeLoadedSettings(next: AppSettings): AppSettings {
    return {
      ...DEFAULT_SETTINGS,
      ...next,
      model: next.model ?? "",
      baseUrl: next.baseUrl ?? "",
    };
  }

  function settingsFingerprint(next: AppSettings): string {
    return JSON.stringify({
      ...next,
      model: next.model ?? "",
      baseUrl: next.baseUrl ?? "",
    });
  }

  function clearSettingsSaveTimer(): void {
    if (settingsSaveTimer) {
      window.clearTimeout(settingsSaveTimer);
      settingsSaveTimer = null;
    }
  }

  async function persistSettings(showFeedback = true): Promise<boolean> {
    clearSettingsSaveTimer();
    isSavingSettings = true;
    try {
      const saved = await invoke<AppSettings>("save_settings", { settings });
      settings = normalizeLoadedSettings(saved);
      lastSavedSettingsFingerprint = settingsFingerprint(settings);
      if (showFeedback) {
        setFeedback("设置已保存");
      }
      return true;
    } catch (error) {
      setError(String(error));
      return false;
    } finally {
      isSavingSettings = false;
    }
  }

  async function bootstrap(): Promise<void> {
    isBootstrapping = true;
    try {
      const payload = await invoke<BootstrapPayload>("bootstrap");
      settings = normalizeLoadedSettings(payload.settings);
      lastSavedSettingsFingerprint = settingsFingerprint(settings);
      settingsHydrated = true;
      balance = payload.balance;
      activeTasks = payload.activeTasks;
      history = payload.history;
      dataDir = payload.dataDir;
      artifactsDir = payload.artifactsDir;
      setFeedback("工作台已就绪");
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
    const saved = await persistSettings(true);
    if (saved && settings.billingAccessKey && settings.billingSecretKey) {
      await refreshBalanceAction(false, false);
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
      setFeedback(`任务 #${created.id} 已加入队列`);
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
    setFeedback("提示词已加载到创作区");
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
    setFeedback("参考素材已加载到创作区");
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
      setFeedback("提示词已拖入创作区");
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
      const unlistenGeneration = await listen<GenerationUpdatedEvent>(
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
      const unlistenBalance = await listen<BalanceUpdatedEvent>(
        "billing-balance-updated",
        ({ payload }) => {
          balance = payload.balance;
        },
      );
      unlisten = () => {
        unlistenGeneration();
        unlistenBalance();
      };
    })();

    return () => {
      unlisten();
      clearSettingsSaveTimer();
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
      setFeedback("首帧已从历史记录载入");
      return;
    }

    if (slot === "last") {
      revokePreview(inputLastFrame);
      inputLastFrame = asset;
      setFeedback("尾帧已从历史记录载入");
      return;
    }

    referenceImages = [...referenceImages, asset];
    setFeedback("参考图已从历史记录追加");
  }

  function amountValue(value?: string | null): number | null {
    if (value == null || value === "") return null;
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }

  function formatAmount(value?: string | null): string {
    const parsed = amountValue(value);
    return parsed == null ? "--" : amountFormatter.format(parsed);
  }

  function isBillingConfigured(): boolean {
    return Boolean(settings.billingAccessKey && settings.billingSecretKey);
  }

  function isLowBalance(): boolean {
    const available = amountValue(balance.availableBalance);
    return available != null && settings.lowBalanceThreshold > 0 && available <= settings.lowBalanceThreshold;
  }

  function hasArrears(): boolean {
    const arrears = amountValue(balance.arrearsBalance);
    return arrears != null && arrears > 0;
  }

  function balanceStatusText(): string {
    if (!isBillingConfigured()) return "请先配置 Billing AK/SK，才能启用余额跟踪。";
    if (hasArrears()) {
      return balance.errorMessage
        ? `账户存在欠费。最近一次刷新失败：${balance.errorMessage}`
        : "账户存在欠费，请先充值再继续消耗额度。";
    }
    if (isLowBalance()) {
      return balance.errorMessage
        ? `可用余额已低于告警阈值（${formatAmount(String(settings.lowBalanceThreshold))}）。最近一次刷新失败：${balance.errorMessage}`
        : `可用余额已低于告警阈值（${formatAmount(String(settings.lowBalanceThreshold))}）。`;
    }
    if (balance.errorMessage) return balance.errorMessage;
    return "余额状态正常。";
  }

  async function refreshBalanceAction(showFeedback = true, persistFirst = true): Promise<void> {
    if (!isBillingConfigured()) {
      if (showFeedback) setError("尚未配置 Billing AK/SK。");
      return;
    }

    if (persistFirst) {
      const saved = await persistSettings(false);
      if (!saved) return;
    }

    isRefreshingBalance = true;
    try {
      balance = await invoke<BalanceSnapshot>("refresh_balance");
      if (showFeedback) setFeedback("余额已刷新");
    } catch (error) {
      setError(String(error));
    } finally {
      isRefreshingBalance = false;
    }
  }

  async function exportSecretBundleAction(): Promise<void> {
    if (!secretBundlePassword) {
      setError("请先输入密码，再导出密钥包。");
      return;
    }

    isExportingSecretBundle = true;
    try {
      secretBundleExport = await invoke<string>("export_secret_bundle", {
        password: secretBundlePassword,
      });
      setFeedback("已生成加密密钥包");
    } catch (error) {
      setError(String(error));
    } finally {
      isExportingSecretBundle = false;
    }
  }

  async function importSecretBundleAction(): Promise<void> {
    if (!secretBundlePassword) {
      setError("请先输入密钥包密码，再执行导入。");
      return;
    }
    if (!secretBundleImport.trim()) {
      setError("请先粘贴导出的密钥包内容。");
      return;
    }

    isImportingSecretBundle = true;
    try {
      settings = await invoke<AppSettings>("import_secret_bundle", {
        password: secretBundlePassword,
        payload: secretBundleImport,
      });
      settings = normalizeLoadedSettings(settings);
      lastSavedSettingsFingerprint = settingsFingerprint(settings);
      if (settings.billingAccessKey && settings.billingSecretKey) {
        await refreshBalanceAction(false, false);
      }
      setFeedback("密钥包已导入到本地密钥存储");
    } catch (error) {
      setError(String(error));
    } finally {
      isImportingSecretBundle = false;
    }
  }

  async function copySecretBundleAction(): Promise<void> {
    if (!secretBundleExport) {
      setError("请先生成密钥包，再复制。");
      return;
    }

    try {
      await navigator.clipboard.writeText(secretBundleExport);
      setFeedback("已复制加密密钥包到剪贴板");
    } catch (error) {
      setError(String(error));
    }
  }

  $: if (settingsHydrated) {
    const nextFingerprint = settingsFingerprint(settings);
    if (nextFingerprint !== lastSavedSettingsFingerprint && !isSavingSettings) {
      clearSettingsSaveTimer();
      settingsSaveTimer = window.setTimeout(() => {
        void persistSettings(false);
      }, 700);
    }
  }
</script>

<svelte:head>
  <title>Seedance Studio 视频工作台</title>
</svelte:head>

<div class="studio-shell">
  <header class="hero">
    <div>
      <p class="eyebrow">Seedance Studio</p>
      <h1>在同一个桌面工作台里完成提示词编辑、历史回看和本地素材管理。</h1>
      <p class="hero-copy">
        应用会把每次生成结果落盘保存，用轻量缩略图预览历史记录，并且允许你不离开当前窗口就把旧提示词和参考素材重新拖回创作区复用。
      </p>
    </div>
    <div class="meta-panel">
      <div>
        <span class="meta-label">数据目录</span>
        <span class="meta-value">{dataDir || "正在初始化..."}</span>
      </div>
      <div>
        <span class="meta-label">素材目录</span>
        <span class="meta-value">{artifactsDir || "正在准备存储..."}</span>
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
          <p class="panel-kicker">创作区</p>
          <h2>创建新的视频任务</h2>
        </div>
        <button class="primary-button" disabled={isSubmitting || isBootstrapping} on:click={submitGeneration}>
          {#if isSubmitting}正在入队...{:else}开始生成{/if}
        </button>
      </div>

      <label class="field">
        <span>提示词</span>
        <textarea
          class="prompt-field"
          placeholder="描述动作、镜头运动、氛围和节奏。"
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
          aria-label="首帧投放区"
          on:dragover={allowDrop}
          on:drop={(event) => handleAssetDrop(event, "first")}
        >
          <div class="field-head">
            <span>首帧</span>
            {#if firstFrame}
              <button class="link-button" on:click={() => { revokePreview(firstFrame); firstFrame = null; }}>
                清空
              </button>
            {/if}
          </div>
          <label class="asset-box">
            {#if firstFrame?.previewUrl}
              <img src={firstFrame.previewUrl} alt="首帧预览" />
            {:else}
              <span>把可复用图片拖到这里，或者上传一张新图片。</span>
            {/if}
            <input accept="image/*" type="file" on:change={(event) => handleSingleFileChange(event, "first")} />
          </label>
        </div>

        <div
          class="field drop-field"
          role="group"
          aria-label="输入尾帧投放区"
          on:dragover={allowDrop}
          on:drop={(event) => handleAssetDrop(event, "last")}
        >
          <div class="field-head">
            <span>输入尾帧</span>
            {#if inputLastFrame}
              <button class="link-button" on:click={() => { revokePreview(inputLastFrame); inputLastFrame = null; }}>
                清空
              </button>
            {/if}
          </div>
          <label class="asset-box">
            {#if inputLastFrame?.previewUrl}
              <img src={inputLastFrame.previewUrl} alt="输入尾帧预览" />
            {:else}
              <span>在生成前先设定最终构图目标。</span>
            {/if}
            <input accept="image/*" type="file" on:change={(event) => handleSingleFileChange(event, "last")} />
          </label>
        </div>

        <div
          class="field drop-field wide"
          role="group"
          aria-label="参考图投放区"
          on:dragover={allowDrop}
          on:drop={(event) => handleAssetDrop(event, "reference")}
        >
          <div class="field-head">
            <span>参考图</span>
            <label class="upload-chip">
              添加
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
                  <button class="remove-button" on:click={() => removeReference(index)}>移除</button>
                </div>
              {/each}
            {:else}
              <p class="reference-placeholder">把历史素材拖到这里会追加到当前列表，不会覆盖已有参考图。</p>
            {/if}
          </div>
        </div>
      </div>

      <div class="controls-grid">
        <label class="field">
          <span>画幅比例</span>
          <select bind:value={form.ratio}>
            {#each ratioOptions as option}
              <option value={option}>{option}</option>
            {/each}
          </select>
        </label>

        <label class="field">
          <span>分辨率</span>
          <select bind:value={form.resolution}>
            {#each resolutionOptions as option}
              <option value={option}>{option}</option>
            {/each}
          </select>
        </label>

        <label class="field">
          <span>时长（秒）</span>
          <input bind:value={form.duration} min="1" step="1" type="number" />
        </label>

        <label class="field">
          <span>帧数</span>
          <input bind:value={form.frames} min="1" step="1" placeholder="可选" type="number" />
        </label>

        <label class="field">
          <span>随机种子</span>
          <input bind:value={form.seed} min="0" step="1" placeholder="可选" type="number" />
        </label>
      </div>

      <div class="toggle-row">
        <label><input bind:checked={form.returnLastFrame} type="checkbox" /> 返回尾帧</label>
        <label><input bind:checked={form.cameraFixed} type="checkbox" /> 固定镜头</label>
        <label><input bind:checked={form.watermark} type="checkbox" /> 添加水印</label>
        <label><input bind:checked={form.generateAudio} type="checkbox" /> 生成音频</label>
        <label><input bind:checked={form.draft} type="checkbox" /> 草稿模式</label>
      </div>
    </div>

    <aside class="panel settings-panel">
      <div class="panel-header">
        <div>
          <p class="panel-kicker">密钥管理</p>
          <h2>凭证与计费</h2>
        </div>
        <button class="secondary-button" disabled={isSavingSettings} on:click={saveSettingsAction}>
          {#if isSavingSettings}正在保存...{:else}保存{/if}
        </button>
      </div>

      <label class="field">
        <span>Seedance API 密钥</span>
        <input bind:value={settings.apiKey} placeholder="输入 ARK API 密钥" type="password" />
      </label>

      <div class="controls-grid two-column">
        <label class="field">
          <span>平台</span>
          <select bind:value={settings.platform}>
            <option value="volc">火山引擎</option>
            <option value="byteplus">BytePlus</option>
          </select>
        </label>

        <label class="field">
          <span>轮询间隔</span>
          <input bind:value={settings.pollInterval} min="1" step="0.5" type="number" />
        </label>
      </div>

      <label class="field">
        <span>模型覆盖</span>
        <input bind:value={settings.model} placeholder="可选" type="text" />
      </label>

      <label class="field">
        <span>Base URL 覆盖</span>
        <input bind:value={settings.baseUrl} placeholder="可选" type="text" />
      </label>

      <div class="subpanel">
        <div class="subpanel-head">
          <div>
            <p class="panel-kicker minor">计费</p>
            <h3>余额查询凭证</h3>
          </div>
          <button class="secondary-button" disabled={isRefreshingBalance} on:click={() => refreshBalanceAction()}>
            {#if isRefreshingBalance}正在刷新...{:else}刷新余额{/if}
          </button>
        </div>

        <label class="field">
          <span>Billing AccessKey</span>
          <input bind:value={settings.billingAccessKey} placeholder="输入火山计费 AccessKey" type="password" />
        </label>

        <label class="field">
          <span>Billing SecretKey</span>
          <input bind:value={settings.billingSecretKey} placeholder="输入火山计费 SecretKey" type="password" />
        </label>

        <label class="field">
          <span>低余额告警阈值</span>
          <input bind:value={settings.lowBalanceThreshold} min="0" step="1" type="number" />
        </label>

        <div class={`balance-banner ${hasArrears() ? "danger" : isLowBalance() ? "warn" : "ok"}`}>
          <strong>{balanceStatusText()}</strong>
          <span>
            {#if balance.updatedAt}
              更新时间：{displayDate(balance.updatedAt)}
            {:else}
              等待首次成功同步余额
            {/if}
          </span>
        </div>

        <div class="balance-grid">
          <div class="balance-card featured">
            <span class="meta-label">可用余额</span>
            <strong>{formatAmount(balance.availableBalance)}</strong>
          </div>
          <div class="balance-card">
            <span class="meta-label">现金余额</span>
            <strong>{formatAmount(balance.cashBalance)}</strong>
          </div>
          <div class="balance-card">
            <span class="meta-label">欠费金额</span>
            <strong>{formatAmount(balance.arrearsBalance)}</strong>
          </div>
          <div class="balance-card">
            <span class="meta-label">信用额度</span>
            <strong>{formatAmount(balance.creditLimit)}</strong>
          </div>
          <div class="balance-card">
            <span class="meta-label">冻结金额</span>
            <strong>{formatAmount(balance.freezeAmount)}</strong>
          </div>
          <div class="balance-card">
            <span class="meta-label">账户 ID</span>
            <strong>{balance.accountId ?? "--"}</strong>
          </div>
        </div>
      </div>

      <div class="settings-note">
        所有密钥只保存在当前机器的应用本地数据库中。这里的配置会自动持久化；余额会在手动刷新、应用启动时，以及每个任务进入终态五秒后自动刷新。
      </div>

      <div class="subpanel">
        <div class="subpanel-head">
          <div>
            <p class="panel-kicker minor">密钥包</p>
            <h3>导出与导入加密密钥</h3>
          </div>
        </div>

        <label class="field">
          <span>密钥包密码</span>
          <input bind:value={secretBundlePassword} placeholder="至少 8 个字符" type="password" />
        </label>

        <div class="secret-actions">
          <button class="secondary-button" disabled={isExportingSecretBundle} on:click={exportSecretBundleAction}>
            {#if isExportingSecretBundle}正在导出...{:else}导出加密密钥包{/if}
          </button>
          <button class="secondary-button" disabled={!secretBundleExport} on:click={copySecretBundleAction}>
            复制导出结果
          </button>
        </div>

        <label class="field">
          <span>导出结果</span>
          <textarea
            class="secret-box"
            bind:value={secretBundleExport}
            placeholder="这里会显示加密后的 base64 密钥包。"
          ></textarea>
        </label>

        <label class="field">
          <span>导入内容</span>
          <textarea
            class="secret-box"
            bind:value={secretBundleImport}
            placeholder="把之前导出的加密 base64 密钥包粘贴到这里。"
          ></textarea>
        </label>

        <button class="secondary-button" disabled={isImportingSecretBundle} on:click={importSecretBundleAction}>
          {#if isImportingSecretBundle}正在导入...{:else}导入到本地密钥存储{/if}
        </button>

        <div class="settings-note">
          导出的字符串会先加密，再编码为 base64 方便传输。真正提供保护的是加密本身，不是 base64。
        </div>
      </div>
    </aside>
  </section>

  <section class="panel active-panel">
    <div class="panel-header compact">
      <div>
        <p class="panel-kicker">实时队列</p>
        <h2>进行中的任务</h2>
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
              <span>{item.progressText ?? "等待远端进度返回..."}</span>
            </div>
            <time>{displayDate(item.updatedAt)}</time>
          </button>
        {/each}
      </div>
    {:else}
      <div class="empty-state small">当前没有进行中的任务。</div>
    {/if}
  </section>

  <section class="panel history-panel">
    <div class="panel-header">
      <div>
        <p class="panel-kicker">历史记录</p>
        <h2>生成记录</h2>
      </div>
      <div class="history-toolbar">
        <select bind:value={statusFilter} on:change={(event) => changeFilter((event.currentTarget as HTMLSelectElement).value)}>
          <option value="">全部状态</option>
          <option value="queued">已排队</option>
          <option value="running">生成中</option>
          <option value="succeeded">已成功</option>
          <option value="failed">已失败</option>
          <option value="cancelled">已取消</option>
          <option value="expired">已过期</option>
        </select>
        <div class="pagination">
          <button class="secondary-button" disabled={history.page <= 1} on:click={() => changePage(history.page - 1)}>
            上一页
          </button>
          <span>第 {history.page} / {totalPages()} 页</span>
          <button class="secondary-button" disabled={history.page >= totalPages()} on:click={() => changePage(history.page + 1)}>
            下一页
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
                <div class="media-fallback">暂无缩略图</div>
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
              <p class="history-progress">{item.progressText ?? item.errorMessage ?? "可直接复用"}</p>
            </div>

            <div class="history-actions">
              <button class="primary-button subtle" on:click={() => reuseGenerationById(item.id)}>整条复用</button>
              <button class="secondary-button" on:click={() => usePrompt(item.prompt)}>复用提示词</button>
              <button class="secondary-button" on:click={() => openDetail(item.id)}>查看详情</button>
              {#if item.videoPath}
                <button class="secondary-button" on:click={() => openSummaryVideo(item)}>显示文件</button>
                <button
                  class="secondary-button"
                  draggable="true"
                  on:dragstart={(event) => startSummaryVideoExportDrag(event, item)}
                >
                  拖出文件
                </button>
              {/if}
            </div>
          </article>
        {/each}
      </div>
    {:else}
      <div class="empty-state">当前筛选条件下还没有历史记录。</div>
    {/if}
  </section>

  {#if drawerOpen && selectedGeneration}
    <aside class="drawer">
      <button
        type="button"
        class="drawer-backdrop"
        aria-label="关闭详情面板"
        on:click={() => (drawerOpen = false)}
      ></button>
      <div class="drawer-panel">
        <div class="panel-header">
          <div>
            <p class="panel-kicker">任务 #{selectedGeneration.id}</p>
            <h2>{statusLabel(selectedGeneration.status)}</h2>
          </div>
          <button class="secondary-button" on:click={() => (drawerOpen = false)}>关闭</button>
        </div>

        <div class="drawer-section">
          <div class="drawer-tools">
            <button class="primary-button subtle" on:click={reuseSelectedGeneration}>整条复用</button>
            <button class="primary-button" on:click={loadSelectedPrompt}>加载提示词</button>
            <button class="secondary-button" on:click={loadSelectedAssets}>加载素材</button>
            {#if selectedGeneration.videoPath}
              <button class="secondary-button" on:click={openSelectedVideo}>显示视频文件</button>
              <button
                class="secondary-button"
                draggable="true"
                on:dragstart={startSelectedVideoExportDrag}
              >
                拖出视频文件
              </button>
            {/if}
          </div>

          <div
            class="prompt-card"
            role="button"
            tabindex="0"
            aria-label="加载提示词或拖回创作区"
            draggable="true"
            on:click={loadSelectedPrompt}
            on:keydown={(event) => (event.key === "Enter" || event.key === " ") && loadSelectedPrompt()}
            on:dragstart={startSelectedPromptDrag}
          >
            <span class="prompt-label">点击即可加载，也可以把这段提示词拖回创作区。</span>
            <p>{selectedGeneration.prompt}</p>
          </div>
        </div>

        <div class="drawer-section">
          <div class="drawer-grid">
            <div class="media-stack large">
              <span>生成视频</span>
              {#if selectedGeneration.videoPath}
                <!-- svelte-ignore a11y_media_has_caption -->
                <video controls playsinline preload="metadata" src={assetSrc(selectedGeneration.videoPath) ?? undefined}></video>
              {:else}
                <div class="media-fallback">视频尚未下载</div>
              {/if}
            </div>

            <div class="media-stack">
              <span>首帧</span>
              {#if selectedGeneration.firstFramePath}
                <button
                  class="asset-button"
                  draggable="true"
                  on:click={() => loadSelectedAssetToSlot("first", selectedGeneration?.firstFramePath)}
                  on:dragstart={(event) => startSelectedAssetDrag(event, selectedGeneration?.firstFramePath, "first")}
                >
                  <img alt="首帧" src={assetSrc(selectedGeneration.firstFramePath) ?? undefined} />
                </button>
              {:else}
                <div class="media-fallback">未设置</div>
              {/if}
            </div>

            <div class="media-stack">
              <span>输入尾帧</span>
              {#if selectedGeneration.inputLastFramePath}
                <button
                  class="asset-button"
                  draggable="true"
                  on:click={() => loadSelectedAssetToSlot("last", selectedGeneration?.inputLastFramePath)}
                  on:dragstart={(event) =>
                    startSelectedAssetDrag(event, selectedGeneration?.inputLastFramePath, "last")}
                >
                  <img alt="输入尾帧" src={assetSrc(selectedGeneration.inputLastFramePath) ?? undefined} />
                </button>
              {:else}
                <div class="media-fallback">未设置</div>
              {/if}
            </div>

            <div class="media-stack">
              <span>返回尾帧</span>
              {#if selectedGeneration.returnedLastFramePath}
                <button
                  class="asset-button"
                  draggable="true"
                  on:click={() => loadSelectedAssetToSlot("last", selectedGeneration?.returnedLastFramePath)}
                  on:dragstart={(event) =>
                    startSelectedAssetDrag(event, selectedGeneration?.returnedLastFramePath, "last")}
                >
                  <img
                    alt="返回尾帧"
                    src={assetSrc(selectedGeneration.returnedLastFramePath) ?? undefined}
                  />
                </button>
              {:else}
                <div class="media-fallback">未返回</div>
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
                <img alt="参考素材" src={assetSrc(imagePath) ?? undefined} />
              </button>
            {/each}
          </div>
        </div>

        <div class="drawer-section info-grid">
          <div>
            <span class="meta-label">任务 ID</span>
            <span class="meta-value">{selectedGeneration.taskId ?? "--"}</span>
          </div>
          <div>
            <span class="meta-label">创建时间</span>
            <span class="meta-value">{displayDate(selectedGeneration.createdAt)}</span>
          </div>
          <div>
            <span class="meta-label">更新时间</span>
            <span class="meta-value">{displayDate(selectedGeneration.updatedAt)}</span>
          </div>
          <div>
            <span class="meta-label">参考图数量</span>
            <span class="meta-value">{selectedGeneration.referenceCount}</span>
          </div>
        </div>

        <div class="drawer-section">
          <span class="meta-label">参数</span>
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

  h3 {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
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

  .panel-kicker.minor {
    margin-bottom: 0.2rem;
    font-size: 0.72rem;
    letter-spacing: 0.12em;
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

  .subpanel {
    display: grid;
    gap: 0.85rem;
    margin-top: 1rem;
    padding: 1rem;
    border-radius: 20px;
    border: 1px solid var(--line);
    background: rgba(8, 23, 20, 0.78);
  }

  .subpanel-head {
    display: flex;
    justify-content: space-between;
    gap: 0.8rem;
    align-items: start;
  }

  .balance-banner {
    display: grid;
    gap: 0.25rem;
    padding: 0.85rem 0.95rem;
    border-radius: 18px;
    border: 1px solid var(--line);
  }

  .balance-banner.ok {
    background: rgba(109, 226, 187, 0.08);
    border-color: rgba(109, 226, 187, 0.2);
  }

  .balance-banner.warn {
    background: rgba(255, 196, 95, 0.08);
    border-color: rgba(255, 196, 95, 0.28);
  }

  .balance-banner.danger {
    background: rgba(255, 142, 111, 0.1);
    border-color: rgba(255, 142, 111, 0.36);
  }

  .balance-banner span {
    color: var(--text-dim);
    font-size: 0.84rem;
  }

  .balance-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0.75rem;
  }

  .balance-card {
    display: grid;
    gap: 0.3rem;
    padding: 0.8rem 0.9rem;
    border-radius: 18px;
    border: 1px solid var(--line);
    background: rgba(10, 22, 20, 0.88);
  }

  .balance-card.featured {
    border-color: rgba(109, 226, 187, 0.26);
    background: linear-gradient(180deg, rgba(16, 46, 40, 0.98), rgba(10, 22, 20, 0.92));
  }

  .balance-card strong {
    font-size: 1.1rem;
  }

  .secret-actions {
    display: flex;
    gap: 0.75rem;
    flex-wrap: wrap;
  }

  .secret-box {
    min-height: 120px;
    resize: vertical;
    padding: 0.85rem 0.95rem;
    border: 1px solid var(--line);
    border-radius: 18px;
    background: rgba(7, 22, 18, 0.9);
    color: var(--text);
    line-height: 1.45;
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
