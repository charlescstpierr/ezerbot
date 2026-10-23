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
  return { id, url, name, caption, mime_type: (typeof attachment !== "string" && attachment.mime_type) || mimeFromName(name) };
}

function inlineCodeRanges(text, offset, ranges) {
  const runs = /`+/g;
  let open;
  while ((open = runs.exec(text))) {
    let slashes = 0;
    while (text[open.index - 1 - slashes] === "\\") slashes += 1;
    const escaped = slashes % 2 === 1;
    const size = open[0].length - (escaped ? 1 : 0);
    if (!size) continue;
    const close = new RegExp(`(?<!\`)\`{${size}}(?!\`)`, "g");
    close.lastIndex = runs.lastIndex;
    const found = close.exec(text);
    const boundary = /\n[ \t]*(?:\n|(?:[-+*]|\d{1,9}[.)])[ \t]|#{1,6}[ \t])/g;
    boundary.lastIndex = runs.lastIndex;
    const gap = boundary.exec(text);
    if (!found || (gap && gap.index < found.index)) continue;
    ranges.push([offset + open.index + (escaped ? 1 : 0), offset + found.index + found[0].length]);
    runs.lastIndex = found.index + found[0].length;
  }
}

function codeRanges(body) {
  const ranges = [];
  let fence = null;
  let start = 0;
  let textStart = 0;
  for (const line of body.split("\n")) {
    const end = start + line.length + 1;
    let rest = line;
    if (fence) {
      const quote = fence.quotes ? line.match(new RegExp(`^[ \\t]*(?:>[ \\t]?){${fence.quotes}}`)) : [""];
      rest = quote ? line.slice(quote[0].length) : null;
      if (rest !== null && fence.indent && rest.trim() && rest.match(/^[ \t]*/)[0].length < fence.indent) rest = null;
      if (rest === null) {
        ranges.push([fence.start, start]);
        textStart = start;
        fence = null;
        rest = line;
      }
    }
    if (!fence) {
      const opening = rest.match(/^((?:[ \t]*(?:>[ \t]?)*))([ \t]*(?:(?:[-+*]|\d{1,9}[.)])[ \t]+)*)(`{3,}|~{3,})(.*)$/);
      if (opening && !(opening[3][0] === "`" && opening[4].includes("`"))) {
        inlineCodeRanges(body.slice(textStart, start), textStart, ranges);
        fence = {
          char: opening[3][0],
          size: opening[3].length,
          start,
          quotes: (opening[1].match(/>/g) || []).length,
          indent: /[-+*\d]/.test(opening[2]) ? opening[2].length : 0,
        };
      }
    } else if (new RegExp(`^[ \\t]*\\${fence.char}{${fence.size},}[ \\t]*$`).test(rest)) {
      ranges.push([fence.start, end]);
      textStart = end;
      fence = null;
    }
    start = end;
  }
  if (fence) ranges.push([fence.start, body.length]);
  else inlineCodeRanges(body.slice(textStart), textStart, ranges);
  return ranges;
}

function imageNodes(body) {
  const ranges = codeRanges(body);
  return [...body.matchAll(IMAGE_MARKDOWN)].filter((match) => !ranges.some(([from, to]) => match.index >= from && match.index < to));
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
    for (const match of imageNodes(message.body)) {
      const url = match[2];
      if (!localImageUrl(url)) continue;
      const caption = match[1].trim() || "Image jointe";
      const name = nameFromUrl(url);
      push({ id: url, url, name, caption, mime_type: mimeFromName(name) });
    }
  }
  return images;
}

export function proseWithoutGallery(body, images) {
  if (!body) return "";
  const urls = new Set((images || []).map((image) => image.url));
  let prose = "";
  let cursor = 0;
  for (const match of imageNodes(body)) {
    if (!urls.has(match[2])) continue;
    prose += body.slice(cursor, match.index);
    cursor = match.index + match[0].length;
    if (!prose.trim()) cursor += body.slice(cursor).match(/^\s*/)[0].length;
  }
  if (cursor === 0) return body;
  const rest = body.slice(cursor);
  return rest.trim() ? prose + rest : prose.trimEnd();
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
  const known = named.match(/\.(png|jpe?g|gif|webp|bmp)$/i);
  if (!known) return `${named}${extension}`;
  if (mimeFromName(named) === type) return named;
  return `${named.slice(0, -known[0].length)}${extension}`;
}

async function loadImageBytes(image, io) {
  if (io.bytes) return { bytes: io.bytes, type: image.mime_type || "image/png" };
  const response = await (io.fetch ?? globalThis.fetch)(image.url);
  if (!response?.ok) throw new Error("Image introuvable.");
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (!bytes.byteLength) throw new Error("Image introuvable.");
  const header = response.headers?.get?.("content-type")?.split(";")[0].trim().toLowerCase();
  const type = Object.hasOwn(EXTENSIONS, header) ? header : image.mime_type || "image/png";
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

async function decodeImage(blob) {
  if (typeof createImageBitmap === "function") return createImageBitmap(blob);
  const url = URL.createObjectURL(blob);
  try {
    const element = new Image();
    element.src = url;
    await element.decode();
    return element;
  } finally {
    URL.revokeObjectURL(url);
  }
}

async function pngBlob(bytes, type, io) {
  if (type === "image/png") return new Blob([bytes], { type });
  try {
    const bitmap = await (io.decode ?? decodeImage)(new Blob([bytes], { type }));
    const canvas = (io.document ?? globalThis.document).createElement("canvas");
    canvas.width = bitmap.naturalWidth || bitmap.width;
    canvas.height = bitmap.naturalHeight || bitmap.height;
    canvas.getContext("2d").drawImage(bitmap, 0, 0);
    bitmap.close?.();
    const png = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
    if (!png) throw new Error("Image illisible.");
    return png;
  } catch {
    throw new Error("Image illisible.");
  }
}

export async function copyGalleryImage(image, io = {}) {
  if (io.native?.postMessage) {
    const { bytes, type } = await loadImageBytes(image, io);
    const result = await io.native.postMessage({ command: "copy", mime: type, data: bytesToBase64(bytes) });
    if (result && result.ok === false) throw new Error("L’image n’a pas pu être copiée.");
    return { type, byteLength: bytes.byteLength };
  }
  const clipboard = io.clipboard ?? globalThis.navigator?.clipboard;
  if (!clipboard?.write) throw new Error("Le presse-papiers n’est pas disponible.");
  const blob = loadImageBytes(image, io).then(({ bytes, type }) => pngBlob(bytes, type, io));
  const loaded = blob.then(() => null, (error) => error);
  const parts = { "image/png": blob };
  try {
    await clipboard.write([typeof ClipboardItem === "function" ? new ClipboardItem(parts) : parts]);
  } catch (error) {
    throw (await loaded) || error;
  }
  const written = await blob;
  return { type: written.type, byteLength: written.size };
}
