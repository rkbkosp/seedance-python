const SETTINGS_KEY = "seedance-studio-draft";

export function loadDraftPrompt(): string {
  return window.localStorage.getItem(SETTINGS_KEY) ?? "";
}

export function saveDraftPrompt(prompt: string): void {
  window.localStorage.setItem(SETTINGS_KEY, prompt);
}

