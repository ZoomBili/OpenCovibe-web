export function downloadText(content: string, filename: string, mimeType = "text/plain"): void {
  const blob = new Blob([content], { type: `${mimeType};charset=utf-8` });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.style.display = "none";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(url);
}

export function openExternal(url: string): void {
  window.open(url, "_blank", "noopener,noreferrer");
}

export function applyBrowserZoom(factor: number): void {
  const style = document.documentElement.style as CSSStyleDeclaration & { zoom: string };
  style.zoom = String(factor);
}
