const SKIPPED_KINDS = new Set(["voice", "document", "meeting"]);
const IMAGE_MARKDOWN = /!\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g;
const EXTENSIONS = {
  "image/png": ".png",
  "image/jpeg": ".jpg",
  "image/gif": ".gif",
  "image/webp": ".webp",
  "image/bmp": ".bmp",
};

function mimeFromName(name) {
  const match = String(name || "").toLowerCase().match(/\.([a-z0-9]+)$/);
  return { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", bmp: "image/bmp" }[match?.[1]] || "";
}

function localImageUrl(url) {
  if (!url || url.startsWith("data:") || url.includes("://") || url.startsWith("//")) return false;
  const path = url.split("?")[0];
  return path.startsWith("/api/attachments/") || path.startsWith("/api/generated/");
}

function safeFileName(value) {
  const cleaned = String(value || "").replace(/[\u0000-\u001f\\/:*?"<>|]/g, " ").replace(/\s+/g, " ").trim();
  return cleaned.replace(/^\.+/, "").slice(0, 120);
}

function nameFromUrl(url) {
  try {
    return safeFileName(decodeURIComponent(url.split("?")[0].split("/").pop() || ""));
  } catch {
    return "";
  }
}

function attachmentImage(attachment) {
  const id = typeof attachment === "string" ? attachment : attachment?.id || attachment?.attachment_id;
  if (!id) return null;
  if (typeof attachment !== "string" && SKIPPED_KINDS.has(attachment.kind)) return null;
  if (typeof attachment === "string" && /^(document_|voice_|meeting_)/.test(id)) return null;
  const name = typeof attachment === "string" ? "" : attachment.name || "";
  const caption = (typeof attachment === "string" ? "" : attachment.caption || attachment.name) || "Image jointe";
  const url = typeof attachment !== "string" && attachment.url ? attachment.url : `/api/attachments/${encodeURIComponent(id)}`;
  return { id, url, name, caption, mime_type: (typeof attachment !== "string" && attachment.mime_type) || mimeFromName(name) || "image/png" };
}

export function galleryImages(message, { markdown = false } = {}) {
  const images = [];
  const seen = new Set();
  const push = (image) => {
    if (!image || seen.has(image.url)) return;
    seen.add(image.url);
    images.push(image);
  };
  for (const attachment of message?.attachments || []) push(attachmentImage(attachment));
  if (markdown && typeof message?.body === "string") {
    for (const match of message.body.matchAll(IMAGE_MARKDOWN)) {
      const url = match[2];
      if (!localImageUrl(url)) continue;
      const caption = match[1].trim() || "Image jointe";
      const name = nameFromUrl(url);
      push({ id: url, url, name, caption, mime_type: mimeFromName(name) || "image/png" });
    }
  }
  return images;
}

export function proseWithoutGallery(body, images) {
  if (!body) return "";
  const urls = new Set((images || []).map((image) => image.url));
  return body
    .replace(IMAGE_MARKDOWN, (whole, _alt, url) => (urls.has(url) ? "" : whole))
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

export function galleryKey(key, index, count) {
  if (!count) return null;
  if (key === "ArrowRight" || key === "ArrowDown") return (index + 1) % count;
  if (key === "ArrowLeft" || key === "ArrowUp") return (index - 1 + count) % count;
  return null;
}

export function imageFilename(image, type = image?.mime_type || "image/png") {
  const extension = EXTENSIONS[type] || ".png";
  const named = safeFileName(image?.name) || (image?.caption && image.caption !== "Image jointe" ? safeFileName(image.caption) : "");
  if (!named) return `image${extension}`;
  if (/\.(png|jpe?g|gif|webp|bmp)$/i.test(named)) return named;
  return `${named}${extension}`;
}

async function loadImageBytes(image, io) {
  if (io.bytes) return { bytes: io.bytes, type: image.mime_type || "image/png" };
  const response = await (io.fetch ?? globalThis.fetch)(image.url);
  if (!response?.ok) throw new Error("Image introuvable.");
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (!bytes.byteLength) throw new Error("Image introuvable.");
  const header = response.headers?.get?.("content-type")?.split(";")[0].trim();
  const type = image.mime_type || (header?.startsWith("image/") ? header : "image/png");
  return { bytes, type };
}

function bytesToBase64(bytes) {
  let binary = "";
  for (let index = 0; index < bytes.length; index += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
  }
  return btoa(binary);
}

export function galleryFailureText(error, fallback) {
  if (!error || error.name === "AbortError") return "";
  const message = typeof error.message === "string" ? error.message.trim() : "";
  if (message === "Image introuvable." || message === "Image illisible." || message === "L’image n’a pas pu être enregistrée." || message === "L’image n’a pas pu être copiée.") return message;
  return fallback;
}

export async function saveGalleryImage(image, io = {}) {
  const { bytes, type } = await loadImageBytes(image, io);
  const name = imageFilename(image, type);
  if (io.native?.postMessage) {
    let result;
    try {
      result = await io.native.postMessage({ command: "save", mime: type, name, data: bytesToBase64(bytes) });
    } catch (error) {
      if (error?.name === "AbortError") return null;
      throw new Error(galleryFailureText(error, "L’image n’a pas pu être enregistrée."));
    }
    if (result?.cancelled) return null;
    if (!result || result.ok === false) {
      const message = typeof result?.error === "string" ? result.error.trim() : "";
      throw new Error(message || "L’image n’a pas pu être enregistrée.");
    }
    return result.name || name;
  }
  if (typeof io.showSaveFilePicker === "function") {
    const handle = await io.showSaveFilePicker({
      suggestedName: name,
      types: [{ description: "Image", accept: { [type]: [EXTENSIONS[type] || ".png"] } }],
    });
    if (!handle) return null;
    const writable = await handle.createWritable();
    await writable.write(new Blob([bytes], { type }));
    await writable.close();
    return handle.name || name;
  }
  if (typeof io.writeFile === "function" && io.destination) {
    await io.writeFile(io.destination, bytes);
    return io.destination;
  }
  const doc = io.document ?? globalThis.document;
  if (!doc?.createElement || !doc.body) throw new Error("L’image n’a pas pu être enregistrée.");
  const url = URL.createObjectURL(new Blob([bytes], { type }));
  const link = doc.createElement("a");
  link.href = url;
  link.download = name;
  link.hidden = true;
  doc.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
  return name;
}

export async function copyGalleryImage(image, io = {}) {
  const { bytes, type } = await loadImageBytes(image, io);
  if (io.native?.postMessage) {
    const result = await io.native.postMessage({ command: "copy", mime: type, data: bytesToBase64(bytes) });
    if (result && result.ok === false) throw new Error("L’image n’a pas pu être copiée.");
    return { type, byteLength: bytes.byteLength };
  }
  const clipboard = io.clipboard ?? globalThis.navigator?.clipboard;
  if (!clipboard?.write) throw new Error("Le presse-papiers n’est pas disponible.");
  const blob = new Blob([bytes], { type });
  let item = { [type]: blob };
  if (typeof ClipboardItem === "function") {
    try { item = new ClipboardItem({ [type]: blob }); } catch { item = { [type]: blob }; }
  }
  await clipboard.write([item]);
  return { type, byteLength: bytes.byteLength };
}
