import type { FileInputPayload } from "./types";

function stripDataUrlPrefix(dataUrl: string): string {
  const commaIndex = dataUrl.indexOf(",");
  return commaIndex >= 0 ? dataUrl.slice(commaIndex + 1) : dataUrl;
}

export async function fileToPayload(file: File): Promise<FileInputPayload> {
  const dataUrl = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error("Failed to read file"));
    reader.onload = () => resolve(String(reader.result));
    reader.readAsDataURL(file);
  });

  return {
    fileName: file.name,
    mimeType: file.type || "application/octet-stream",
    base64Data: stripDataUrlPrefix(dataUrl),
    previewUrl: URL.createObjectURL(file),
  };
}

