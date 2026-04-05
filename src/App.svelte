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
    model: "doubao-seedance-1-5-pro-251215",
    baseUrl: "",
    pollInterval: 3,
    billingAccessKey: "",
    billingSecretKey: "",
    billingSecurityToken: "",
    lowBalanceThreshold: 100,
  };

  const ratioOptions = ["16:9", "4:3", "1:1", "3:4", "9:16", "21:9", "adaptive"];
  const resolutionOptions = ["480p", "720p", "1080p"];
  const modelOptions = [
    "doubao-seedance-1-5-pro-251215",
    "doubao-seedance-1-0-pro-250528",
    "doubao-seedance-1-0-pro-fast-251015",
  ];
  const durationOptions = [4, 5, 6, 7, 8, 9, 10, 11, 12];

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
    const labels: Record<string, string> = {
      queued: "已排队",
      running: "生成中",
      succeeded: "已成功",
      failed: "已失败",
      cancelled: "已取消",
      expired: "已过期",
    };
    return labels[status] ?? status.replaceAll("_", " ");
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
      model: next.model || modelOptions[0],
      baseUrl: next.baseUrl ?? "",
    };
  }

  function settingsFingerprint(next: AppSettings): string {
    return JSON.stringify({
      ...next,
      model: next.model || modelOptions[0],
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
    if (!isBillingConfigured()) return "";
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
    if (persistFirst) {
      const saved = await persistSettings(false);
      if (!saved) return;
    }

    if (!isBillingConfigured()) {
      if (showFeedback) setError("尚未配置 Billing AK/SK。");
      return;
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
  <div class="workspace-shell">
    <!-- 左侧边栏：计费和余额。 -->
    <aside class="sidebar sidebar-billing">
      <section class="sidebar-card">
        <div class="sidebar-header">
          <div>
            <p class="panel-kicker minor">计费</p>
            <h2>余额</h2>
          </div>
          <button class="secondary-button compact" disabled={isRefreshingBalance} on:click={() => refreshBalanceAction()}>
            {#if isRefreshingBalance}刷新中{:else}刷新{/if}
          </button>
        </div>

        {#if isBillingConfigured() || balance.updatedAt || balance.errorMessage}
          <div class={`balance-banner ${hasArrears() ? "danger" : isLowBalance() ? "warn" : "ok"}`}>
            <strong>{balanceStatusText()}</strong>
            <span>
              {#if balance.updatedAt}
                更新时间：{displayDate(balance.updatedAt)}
              {:else}
                等待首次同步
              {/if}
            </span>
          </div>
        {/if}

        <div class="balance-grid">
          <div class="balance-card featured">
            <span class="meta-label">可用余额</span>
            <strong>{formatAmount(balance.availableBalance)}</strong>
          </div>
          <div class="balance-card">
            <span class="meta-label">欠费金额</span>
            <strong>{formatAmount(balance.arrearsBalance)}</strong>
          </div>
        </div>

        <div class="field">
          <span>Billing AccessKey</span>
          <input bind:value={settings.billingAccessKey} placeholder="输入火山计费 AccessKey" type="password" />
        </div>

        <div class="field">
          <span>Billing SecretKey</span>
          <input bind:value={settings.billingSecretKey} placeholder="输入火山计费 SecretKey" type="password" />
        </div>
        <div class="field">
          <span>低余额告警阈值</span>
          <input bind:value={settings.lowBalanceThreshold} min="0" step="1" type="number" />
        </div>
      </section>
    </aside>

    <!-- 中间主区域：直接显示历史任务。 -->
    <main class="history-surface">
      {#if activeTasks.length}
        <section class="surface-card active-strip">
          {#each activeTasks as item}
            <button class="active-pill" on:click={() => openDetail(item.id)}>
              <span class="spinner-ring"></span>
              <span class={`status-chip status-${item.status}`}>{statusLabel(item.status)}</span>
              <strong>{item.promptSummary}</strong>
            </button>
          {/each}
        </section>
      {/if}

      <section class="surface-card history-toolbar">
        <div class="toolbar-title">
          <p class="panel-kicker minor">历史</p>
          <h2>任务记录</h2>
        </div>
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
          <button class="secondary-button compact" disabled={history.page <= 1} on:click={() => changePage(history.page - 1)}>
            上一页
          </button>
          <span>第 {history.page} / {totalPages()} 页</span>
          <button class="secondary-button compact" disabled={history.page >= totalPages()} on:click={() => changePage(history.page + 1)}>
            下一页
          </button>
        </div>
      </section>

      {#if history.items.length}
        <section class="history-list">
          {#each history.items as item}
            <article class="surface-card history-row">
              <button
                class="history-thumb"
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

              <div class="history-copy">
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

              <div class="history-actions compact-stack">
                <button class="primary-button subtle" on:click={() => reuseGenerationById(item.id)}>整条复用</button>
                <button class="secondary-button compact" on:click={() => openDetail(item.id)}>详情</button>
                {#if item.videoPath}
                  <button class="secondary-button compact" on:click={() => openSummaryVideo(item)}>显示文件</button>
                {/if}
              </div>
            </article>
          {/each}
        </section>
      {:else}
        <section class="surface-card empty-state">当前筛选条件下还没有历史记录。</section>
      {/if}
    </main>

    <!-- 右侧边栏：项目连接凭证和密钥包导入导出。 -->
    <aside class="sidebar sidebar-credentials">
      <section class="sidebar-card">
        <div class="sidebar-header">
          <div>
            <p class="panel-kicker minor">凭证</p>
            <h2>连接设置</h2>
          </div>
          <button class="secondary-button compact" disabled={isSavingSettings} on:click={saveSettingsAction}>
            {#if isSavingSettings}保存中{:else}保存{/if}
          </button>
        </div>

        <div class="field">
          <span>Seedance API 密钥</span>
          <input bind:value={settings.apiKey} placeholder="输入 ARK API 密钥" type="password" />
        </div>

        <div class="field">
          <span>平台</span>
          <select bind:value={settings.platform}>
            <option value="volc">火山引擎</option>
            <option value="byteplus">BytePlus</option>
          </select>
        </div>

        <div class="field">
          <span>轮询间隔</span>
          <input bind:value={settings.pollInterval} min="1" step="0.5" type="number" />
        </div>

        <div class="field">
          <span>模型覆盖</span>
          <select bind:value={settings.model}>
            {#each modelOptions as option}
              <option value={option}>{option}</option>
            {/each}
          </select>
        </div>
      </section>

      <section class="sidebar-card">
        <div class="sidebar-header">
          <div>
            <p class="panel-kicker minor">密钥包</p>
            <h2>导出 / 导入</h2>
          </div>
        </div>

        <div class="field">
          <span>密钥包密码</span>
          <input bind:value={secretBundlePassword} placeholder="至少 8 个字符" type="password" />
        </div>

        <div class="secret-actions">
          <button class="secondary-button compact" disabled={isExportingSecretBundle} on:click={exportSecretBundleAction}>
            {#if isExportingSecretBundle}导出中{:else}导出{/if}
          </button>
          <button class="secondary-button compact" disabled={!secretBundleExport} on:click={copySecretBundleAction}>
            复制
          </button>
        </div>

        <label class="field">
          <span>导出结果</span>
          <textarea class="secret-box" bind:value={secretBundleExport} placeholder="这里会显示加密后的 base64 密钥包。"></textarea>
        </label>

        <label class="field">
          <span>导入内容</span>
          <textarea class="secret-box" bind:value={secretBundleImport} placeholder="把之前导出的加密 base64 密钥包粘贴到这里。"></textarea>
        </label>

        <button class="secondary-button" disabled={isImportingSecretBundle} on:click={importSecretBundleAction}>
          {#if isImportingSecretBundle}导入中{:else}导入到本地密钥存储{/if}
        </button>

        <p class="settings-note">导出的字符串先加密，再编码为 base64，方便在不同机器之间安全迁移。</p>
      </section>
    </aside>
  </div>

  <!-- 底部悬浮输入区：像聊天框一样发起新任务。 -->
  <section class="composer-dock">
    <div class="composer-card">
      <!-- 第一行：大提示词输入框 + 发送按钮。 -->
      <div class="composer-top">
        <textarea
          class="chat-input"
          placeholder="像聊天一样输入视频提示词..."
          value={form.prompt}
          on:input={(event) => updatePromptDraft((event.currentTarget as HTMLTextAreaElement).value)}
          on:dragover={allowDrop}
          on:drop={handlePromptDrop}
        ></textarea>
        <button class="primary-button composer-send" disabled={isSubmitting || isBootstrapping} on:click={submitGeneration}>
          {#if isSubmitting}发送中{:else}发送生成{/if}
        </button>
      </div>

      <!-- 第二行：首尾帧小框 + 紧凑参数。 -->
      <div class="composer-bottom">
        <div class="composer-inline">
          <div class="frame-inline">
            <div class="frame-inline-item" role="group" aria-label="首帧投放区" on:dragover={allowDrop} on:drop={(event) => handleAssetDrop(event, "first")}>
              <span>首帧</span>
              <label class="inline-upload">
                {#if firstFrame?.previewUrl}
                  <img src={firstFrame.previewUrl} alt="首帧预览" />
                {:else}
                  <span>[上传]</span>
                {/if}
                <input accept="image/*" type="file" on:change={(event) => handleSingleFileChange(event, "first")} />
              </label>
            </div>

            <div class="frame-inline-item" role="group" aria-label="输入尾帧投放区" on:dragover={allowDrop} on:drop={(event) => handleAssetDrop(event, "last")}>
              <span>尾帧</span>
              <label class="inline-upload">
                {#if inputLastFrame?.previewUrl}
                  <img src={inputLastFrame.previewUrl} alt="输入尾帧预览" />
                {:else}
                  <span>[上传]</span>
                {/if}
                <input accept="image/*" type="file" on:change={(event) => handleSingleFileChange(event, "last")} />
              </label>
            </div>
          </div>

          <div class="inline-separator" aria-hidden="true"></div>

          <div class="composer-controls inline-controls">
            <label class="compact-field">
              <span>画幅</span>
              <select bind:value={form.ratio}>
                {#each ratioOptions as option}
                  <option value={option}>{option}</option>
                {/each}
              </select>
            </label>

            <label class="compact-field">
              <span>分辨率</span>
              <select bind:value={form.resolution}>
                {#each resolutionOptions as option}
                  <option value={option}>{option}</option>
                {/each}
              </select>
            </label>

            <label class="compact-field">
              <span>时长</span>
              <select bind:value={form.duration}>
                {#each durationOptions as seconds}
                  <option value={seconds}>{seconds}s</option>
                {/each}
              </select>
            </label>
          </div>

          <div class="inline-separator" aria-hidden="true"></div>

          <label class="mini-check audio-check">
            <input bind:checked={form.generateAudio} type="checkbox" />
            <span>生成音频</span>
          </label>
        </div>
      </div>
    </div>
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
  /* ===== 新手最常改的尺寸都集中在这里 =====
     1. 想调左侧栏宽度：改 --sidebar-left-width
     2. 想调右侧栏宽度：改 --sidebar-right-width
     3. 想调历史缩略图大小：改 --history-thumb-width / --history-thumb-height
     4. 想调底部首尾帧小框大小：改 --composer-frame-width / --composer-frame-height
     5. 想调底部聊天框高度：改 --chat-input-height
  */
  .studio-shell {
    --sidebar-left-width: 220px;
    --sidebar-right-width: 240px;
    --history-thumb-width: 132px;
    --history-thumb-height: 92px;
    --composer-frame-width: 92px;
    --composer-frame-height: 52px;
    --chat-input-height: 78px;
    padding: 1rem;
  }

  .workspace-shell {
    display: grid;
    grid-template-columns: var(--sidebar-left-width) minmax(0, 1fr) var(--sidebar-right-width);
    gap: 1rem;
    align-items: start;
    min-height: calc(100vh - 2rem);
  }

  .sidebar {
    position: sticky;
    top: 1rem;
    display: grid;
    gap: 1rem;
    max-height: calc(100vh - 2rem);
    overflow: auto;
  }

  .history-surface {
    display: grid;
    gap: 1rem;
    padding-bottom: 16rem;
  }

  .surface-card,
  .sidebar-card,
  .composer-card,
  .drawer-panel {
    background: #ffffff;
    border: 1px solid #d9e2ec;
    border-radius: 24px;
    box-shadow: 0 18px 44px rgba(15, 23, 42, 0.08);
  }

  .surface-card,
  .sidebar-card,
  .drawer-panel {
    padding: 1rem;
  }

  .eyebrow,
  .panel-kicker {
    margin: 0 0 0.25rem;
    color: #0c8f68;
    font-size: 0.74rem;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
  }

  .panel-kicker.minor {
    font-size: 0.68rem;
  }

  h2 {
    margin: 0;
    color: #172b4d;
    font-family: "Iowan Old Style", "Palatino Linotype", Georgia, serif;
  }

  h2 {
    font-size: 1rem;
    line-height: 1.08;
  }

  .surface-subtitle,
  .settings-note,
  .history-progress,
  .meta-value,
  .meta-label,
  .field span,
  .prompt-label,
  .media-stack > span,
  .mini-head,
  .compact-field span {
    color: #66758c;
  }

  .surface-header {
    display: grid;
    gap: 1rem;
  }

  .surface-meta {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 0.75rem;
  }

  .meta-box {
    display: grid;
    gap: 0.2rem;
    padding: 0.85rem 0.95rem;
    border-radius: 18px;
    background: #f7f9fc;
    border: 1px solid #e3eaf2;
  }

  .meta-box.status-box.success {
    border-color: #bfe8d9;
    color: #0c7f5d;
  }

  .meta-box.status-box.error {
    border-color: #f6c2bf;
    color: #b42318;
  }

  .sidebar-header,
  .panel-header,
  .history-toolbar,
  .pagination,
  .history-meta,
  .drawer-tools,
  .secret-actions {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.75rem;
    flex-wrap: wrap;
  }

  .field {
    display: grid;
    gap: 0.35rem;
  }

  input,
  select,
  textarea,
  pre {
    width: 100%;
    border-radius: 16px;
    border: 1px solid #d9e2ec;
    background: #ffffff;
    color: #172b4d;
    padding: 0.62rem 0.72rem;
    box-shadow: inset 0 1px 2px rgba(15, 23, 42, 0.03);
    font-size: 0.88rem;
  }

  textarea,
  pre {
    line-height: 1.5;
  }

  button {
    border: 0;
    cursor: pointer;
  }

  .primary-button,
  .secondary-button,
  .mini-link {
    border-radius: 999px;
    padding: 0.55rem 0.85rem;
    transition: transform 120ms ease, box-shadow 120ms ease;
    font-size: 0.84rem;
  }

  .compact {
    padding: 0.5rem 0.85rem;
    font-size: 0.88rem;
  }

  .primary-button {
    background: #0c8f68;
    color: #ffffff;
    font-weight: 700;
    box-shadow: 0 10px 24px rgba(12, 143, 104, 0.18);
  }

  .primary-button.subtle {
    background: #e9f8f2;
    color: #0c7f5d;
    box-shadow: none;
  }

  .secondary-button,
  .mini-link {
    background: #eef3f8;
    color: #304560;
  }

  .primary-button:hover,
  .secondary-button:hover,
  .mini-link:hover {
    transform: translateY(-1px);
  }

  .active-strip {
    display: flex;
    gap: 0.75rem;
    overflow: auto;
  }

  .active-pill {
    display: inline-flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.65rem 0.85rem;
    border-radius: 999px;
    background: #f8fbff;
    border: 1px solid #dce6f2;
    color: #172b4d;
    white-space: nowrap;
  }

  .spinner-ring {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 2px solid #d6efe6;
    border-top-color: #0c8f68;
    animation: spin 0.85s linear infinite;
  }

  .history-list {
    display: grid;
    gap: 0.85rem;
  }

  .history-row {
    display: grid;
    grid-template-columns: var(--history-thumb-width) minmax(0, 1fr) 118px;
    gap: 0.75rem;
    align-items: center;
  }

  .history-thumb {
    position: relative;
    min-height: var(--history-thumb-height);
    padding: 0;
    overflow: hidden;
    border-radius: 16px;
    background: #eef3f8;
  }

  .history-thumb img,
  .history-thumb video,
  .media-stack img,
  .media-stack video,
  .asset-button img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .floating {
    position: absolute;
    top: 0.6rem;
    left: 0.6rem;
    z-index: 1;
  }

  .status-chip {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0.28rem 0.62rem;
    border-radius: 999px;
    font-size: 0.68rem;
    font-weight: 700;
  }

  .status-succeeded {
    background: #e9f8f2;
    color: #0f7d5b;
  }

  .status-failed,
  .status-cancelled,
  .status-expired {
    background: #fdecec;
    color: #b42318;
  }

  .status-running,
  .status-queued {
    background: #eef6ff;
    color: #2456a5;
  }

  .history-copy {
    display: grid;
    gap: 0.3rem;
    min-width: 0;
  }

  .history-prompt {
    padding: 0;
    background: transparent;
    color: #172b4d;
    text-align: left;
    line-height: 1.35;
    font-size: 0.92rem;
  }

  .compact-stack {
    display: grid;
    gap: 0.35rem;
  }

  .toolbar-title {
    display: grid;
    gap: 0.1rem;
  }

  .balance-banner {
    display: grid;
    gap: 0.25rem;
    padding: 0.7rem 0.8rem;
    border-radius: 16px;
    border: 1px solid #dce6f2;
    background: #f8fbff;
  }

  .balance-banner.ok {
    border-color: #bce5d6;
    background: #f1fbf7;
  }

  .balance-banner.warn {
    border-color: #f7d89f;
    background: #fff9eb;
  }

  .balance-banner.danger {
    border-color: #f3c0ba;
    background: #fff2f1;
  }

  .balance-banner span {
    font-size: 0.74rem;
    color: #66758c;
  }

  .balance-grid {
    display: grid;
    grid-template-columns: 1fr;
    gap: 0.55rem;
  }

  .balance-card {
    display: grid;
    gap: 0.2rem;
    padding: 0.68rem 0.75rem;
    border-radius: 14px;
    background: #f8fafc;
    border: 1px solid #e2e8f0;
  }

  .balance-card.featured {
    background: #eefaf5;
    border-color: #c7eadc;
  }

  .balance-card strong {
    font-size: 0.96rem;
    color: #172b4d;
  }

  .secret-box {
    min-height: 100px;
    resize: vertical;
  }

  .composer-dock {
    position: fixed;
    left: clamp(1rem, 13vw, 14.5rem);
    right: clamp(1rem, 14vw, 15.5rem);
    bottom: 1rem;
    z-index: 30;
  }

  .composer-card {
    padding: 0.8rem;
    background: rgba(255, 255, 255, 0.72);
    backdrop-filter: blur(18px) saturate(1.2);
    -webkit-backdrop-filter: blur(18px) saturate(1.2);
    border: 1px solid rgba(217, 226, 236, 0.9);
    box-shadow: 0 18px 42px rgba(23, 43, 77, 0.14);
  }

  .composer-top {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 0.85rem;
    align-items: end;
  }

  .chat-input {
    min-height: var(--chat-input-height);
    resize: none;
    border-radius: 18px;
    background: rgba(255, 255, 255, 0.78);
  }

  .composer-send {
    min-width: 98px;
    align-self: stretch;
  }

  .composer-bottom {
    display: flex;
    justify-content: center;
    margin-top: 0.85rem;
  }

  .composer-inline {
    display: flex;
    align-items: center;
    justify-content: center;
    flex-wrap: wrap;
    gap: 0.55rem;
  }

  .frame-inline {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .frame-inline-item {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.76rem;
    color: #52657d;
  }

  .inline-upload {
    position: relative;
    display: grid;
    place-items: center;
    width: var(--composer-frame-width);
    min-height: var(--composer-frame-height);
    padding: 0.2rem;
    border-radius: 12px;
    background: rgba(248, 250, 252, 0.78);
    border: 1px dashed #ccd7e5;
    overflow: hidden;
    color: #52657d;
    font-size: 0.76rem;
  }

  .inline-upload img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .inline-upload input {
    position: relative;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }

  .inline-separator {
    width: 1px;
    align-self: stretch;
    min-height: 28px;
    background: #d9e2ec;
  }

  .composer-controls {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    justify-content: center;
    flex-wrap: wrap;
  }

  .compact-field {
    display: grid;
    gap: 0.25rem;
    min-width: 84px;
  }

  .compact-field select {
    padding: 0.45rem 0.6rem;
    border-radius: 14px;
    background: #f8fafc;
  }

  .inline-controls {
    align-items: end;
  }

  .mini-check {
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
    padding: 0.45rem 0.6rem;
    border-radius: 14px;
    background: rgba(248, 250, 252, 0.82);
    border: 1px solid #dce4ef;
    color: #304560;
    font-size: 0.76rem;
  }

  .audio-check {
    white-space: nowrap;
  }

  .drawer {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: flex;
    justify-content: flex-end;
  }

  .drawer-backdrop {
    position: absolute;
    inset: 0;
    background: rgba(15, 23, 42, 0.24);
  }

  .drawer-panel {
    position: relative;
    width: min(920px, calc(100vw - 2rem));
    height: 100%;
    overflow: auto;
    background: #ffffff;
  }

  .drawer-section {
    display: grid;
    gap: 0.85rem;
    margin-top: 1rem;
  }

  .prompt-card,
  pre {
    margin: 0;
    padding: 1rem;
    border-radius: 18px;
    background: #f8fafc;
    border: 1px solid #e2e8f0;
    overflow: auto;
  }

  .prompt-card {
    cursor: grab;
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
    gap: 0.45rem;
  }

  .media-stack.large {
    grid-row: span 2;
  }

  .media-stack video,
  .media-stack img,
  .asset-button,
  .media-fallback {
    min-height: 150px;
    border-radius: 18px;
    border: 1px solid #e2e8f0;
    background: #f8fafc;
  }

  .asset-button {
    padding: 0;
    overflow: hidden;
  }

  .media-fallback {
    display: grid;
    place-items: center;
    color: #66758c;
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
    border-radius: 16px;
    background: #f8fafc;
    border: 1px solid #e2e8f0;
  }

  .empty-state {
    display: grid;
    place-items: center;
    min-height: 180px;
    color: #66758c;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (max-width: 1380px) {
    .workspace-shell {
      grid-template-columns: 210px minmax(0, 1fr) 230px;
    }

    .composer-dock {
      left: 13rem;
      right: 14rem;
    }
  }

  @media (max-width: 1180px) {
    .workspace-shell {
      grid-template-columns: 1fr;
    }

    .sidebar {
      position: static;
      max-height: none;
      overflow: visible;
    }

    .composer-dock {
      left: 1rem;
      right: 1rem;
    }

    .surface-meta,
    .history-row,
    .drawer-grid,
    .info-grid {
      grid-template-columns: 1fr;
    }

    .inline-separator {
      display: none;
    }
  }

  @media (max-width: 760px) {
    .studio-shell {
      padding: 0.75rem;
    }

    .history-surface {
      padding-bottom: 18rem;
    }

    .frame-pair {
      width: 100%;
      justify-content: space-between;
    }

    .frame-mini {
      width: calc(50% - 0.4rem);
    }

    .composer-top {
      grid-template-columns: 1fr;
    }

    .composer-send {
      width: 100%;
    }

    .drawer-panel {
      width: 100vw;
    }
  }
</style>
