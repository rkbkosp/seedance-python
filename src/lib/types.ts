export type GenerationStatus =
  | "queued"
  | "running"
  | "succeeded"
  | "failed"
  | "cancelled"
  | "expired";

export type FileInputPayload = {
  existingPath?: string | null;
  fileName?: string | null;
  mimeType?: string | null;
  base64Data?: string | null;
  previewUrl?: string | null;
};

export type AppSettings = {
  apiKey: string;
  platform: "volc" | "byteplus";
  model?: string | null;
  baseUrl?: string | null;
  pollInterval: number;
};

export type GenerationSummary = {
  id: number;
  taskId?: string | null;
  status: GenerationStatus;
  prompt: string;
  promptSummary: string;
  createdAt: string;
  updatedAt: string;
  completedAt?: string | null;
  errorMessage?: string | null;
  progressText?: string | null;
  videoPath?: string | null;
  thumbnailPath?: string | null;
  firstFramePath?: string | null;
  inputLastFramePath?: string | null;
  returnedLastFramePath?: string | null;
  referenceCount: number;
};

export type GenerationDetail = GenerationSummary & {
  paramsJson: string;
  platform: string;
  model: string;
  referenceImages: string[];
};

export type HistoryPage = {
  items: GenerationSummary[];
  page: number;
  pageSize: number;
  total: number;
};

export type BootstrapPayload = {
  settings: AppSettings;
  activeTasks: GenerationSummary[];
  history: HistoryPage;
  dataDir: string;
  artifactsDir: string;
};

export type CreateGenerationRequest = {
  prompt: string;
  firstFrame?: FileInputPayload | null;
  lastFrame?: FileInputPayload | null;
  referenceImages: FileInputPayload[];
  ratio?: string | null;
  resolution?: string | null;
  duration?: number | null;
  frames?: number | null;
  returnLastFrame?: boolean | null;
  draft?: boolean | null;
  cameraFixed?: boolean | null;
  watermark?: boolean | null;
  generateAudio?: boolean | null;
  seed?: number | null;
};

export type GenerationUpdatedEvent = {
  generationId: number;
};
