import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { after, test } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

const PNG = Uint8Array.from(Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==",
  "base64",
));

function image(id, caption) {
  return {
    id: `attachment_${id.repeat(64)}`,
    name: `${caption}.png`,
    caption,
    mime_type: "image/png",
    url: `/api/attachments/${caption}`,
    byte_length: PNG.byteLength,
  };
}

const three = {
  id: "m-trois",
  role: "user",
  state: "accepted",
  body: "Trois photos du port",
  attachments: [image("a", "Phare au crépuscule"), image("b", "Le quai"), image("c", "Ciel violet")],
};

test("the app menus do not gain a Galerie entry", () => {
  const shell = readFileSync(new URL("../../ui/index.html", import.meta.url), "utf8");
  const conversationMenu = shell.slice(shell.indexOf('id="conversation-menu"'), shell.indexOf('id="chat-scroll"'));
  const settings = shell.slice(shell.indexOf('id="settings-dialog"'), shell.indexOf('id="rename-dialog"'));
  assert.equal(conversationMenu.includes("Galerie"), false);
  assert.equal(settings.includes("Galerie"), false);
  const native = readFileSync(new URL("../../native/main.m", import.meta.url), "utf8");
  const menus = native.slice(native.indexOf("- (void)createMenus"), native.indexOf("- (void)showFloating"));
  assert.equal(menus.includes("Galerie"), false);
});

const vite = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)),
  configFile: false,
  plugins: [react()],
  server: { middlewareMode: true },
  logLevel: "error",
  optimizeDeps: { noDiscovery: true },
});
after(() => vite.close());

let gallery;
let view;
try {
  gallery = await vite.ssrLoadModule("/src/image-gallery.js");
  view = await vite.ssrLoadModule("/src/image-gallery.jsx");
} catch (error) {
  await vite.close();
  throw error;
}

function markup(message, props = {}) {
  return renderToStaticMarkup(React.createElement(view.MessageAttachments, { message, ...props }));
}

test("three images of one message render as a grid, and the viewer shows the caption, arrows, save and copy", () => {
  const html = markup(three, { openIndex: 0 });
  const grid = html.match(/class="gb-image-grid[^"]*" data-count="3"/);
  assert.ok(grid, html);
  assert.equal((html.match(/class="gb-image-cell"/g) || []).length, 3);
  for (const caption of ["Phare au crépuscule", "Le quai", "Ciel violet"]) assert.ok(html.includes(caption), caption);
  assert.match(html, /Enregistrer l’image…/);
  assert.match(html, /Copier l’image/);
  assert.match(html, /aria-label="Enregistrer l’image…"/);
  assert.match(html, /aria-label="Copier l’image"/);
  assert.match(html, /aria-label="Image précédente"/);
  assert.match(html, /aria-label="Image suivante"/);
  assert.match(html, /gb-image-viewer/);
  assert.equal(html.includes("Galerie"), false);
  const css = readFileSync(new URL("./chat.css", import.meta.url), "utf8");
  assert.match(css, /\.gb-image-grid\s*\{[^}]*display:\s*grid/);
  const thread = readFileSync(new URL("./Thread.jsx", import.meta.url), "utf8");
  assert.match(thread, /MessageAttachments/);
  assert.match(thread, /markdown/);
});

test("arrow keys and arrow buttons circulate through the three images", () => {
  assert.equal(gallery.galleryKey("ArrowRight", 2, 3), 0);
  assert.equal(gallery.galleryKey("ArrowLeft", 0, 3), 2);
  assert.equal(gallery.galleryKey("ArrowDown", 1, 3), 2);
  assert.equal(gallery.galleryKey("ArrowUp", 0, 3), 2);
  assert.equal(gallery.galleryKey("Enter", 1, 3), null);
  const next = markup(three, { openIndex: gallery.galleryKey("ArrowRight", 0, 3) });
  assert.match(next, /Le quai/);
  assert.doesNotMatch(next.split("gb-image-caption")[1] || "", /Phare au crépuscule/);
  const previous = markup(three, { openIndex: gallery.galleryKey("ArrowLeft", 0, 3) });
  assert.match(previous, /Ciel violet/);
});

test("save writes the image file and copy places that image on the clipboard", async () => {
  const target = three.attachments[0];
  const fetchImage = async (url) => {
    assert.equal(url, target.url);
    return new Response(PNG, { status: 200, headers: { "content-type": "image/png" } });
  };
  const dir = await mkdtemp(join(tmpdir(), "ezerbot-galerie-"));
  const destination = join(dir, "Phare au crépuscule.png");
  let written = null;
  const saved = await gallery.saveGalleryImage(target, {
    fetch: fetchImage,
    showSaveFilePicker: async ({ suggestedName }) => {
      assert.equal(suggestedName, "Phare au crépuscule.png");
      return {
        name: suggestedName,
        async createWritable() {
          return {
            async write(blob) { written = new Uint8Array(await blob.arrayBuffer()); },
            async close() { await writeFile(destination, written); },
          };
        },
      };
    },
  });
  assert.equal(saved, "Phare au crépuscule.png");
  assert.deepEqual(await readFile(destination), Buffer.from(PNG));

  const clipboard = [];
  await gallery.copyGalleryImage(target, {
    fetch: fetchImage,
    clipboard: { async write(items) { clipboard.push(items); } },
  });
  assert.equal(clipboard.length, 1);
  const item = clipboard[0][0];
  const blob = await (typeof item.getType === "function" ? item.getType("image/png") : item["image/png"]);
  assert.ok(blob, "le presse-papiers doit contenir une image/png");
  assert.deepEqual(new Uint8Array(await blob.arrayBuffer()), PNG);
});

test("macOS save asks the shell for a panel before any write, and a failure stays a French error", async () => {
  const target = three.attachments[0];
  const dir = await mkdtemp(join(tmpdir(), "ezerbot-galerie-"));
  const destination = join(dir, "Phare au crépuscule.png");
  const order = [];
  let anchors = 0;
  let picker = 0;
  const saved = await gallery.saveGalleryImage(target, {
    bytes: PNG,
    showSaveFilePicker() { picker += 1; },
    document: {
      body: { append() {} },
      createElement() { anchors += 1; return { style: {}, click() {}, remove() {} }; },
    },
    native: {
      async postMessage(body) {
        order.push("panel");
        assert.equal(body.command, "save");
        assert.equal(body.mime, "image/png");
        assert.equal(body.name, "Phare au crépuscule.png");
        assert.equal(body.data, Buffer.from(PNG).toString("base64"));
        order.push("write");
        await writeFile(destination, Buffer.from(body.data, "base64"));
        return { ok: true, name: body.name };
      },
    },
  });
  assert.deepEqual(order, ["panel", "write"]);
  assert.equal(picker, 0);
  assert.equal(anchors, 0);
  assert.equal(saved, "Phare au crépuscule.png");
  assert.deepEqual(await readFile(destination), Buffer.from(PNG));

  await assert.rejects(
    gallery.saveGalleryImage(target, {
      bytes: PNG,
      native: { async postMessage() { return { ok: false, error: "L’image n’a pas pu être enregistrée." }; } },
    }),
    (error) => {
      assert.equal(gallery.galleryFailureText(error, "autre"), "L’image n’a pas pu être enregistrée.");
      return true;
    },
  );
  assert.equal(await gallery.saveGalleryImage(target, {
    bytes: PNG,
    native: { async postMessage() { return { ok: true, cancelled: true }; } },
  }), null);
  assert.equal(gallery.galleryFailureText({ name: "AbortError", message: "The user aborted a request." }, "L’image n’a pas pu être enregistrée."), "");
  assert.equal(gallery.galleryFailureText(new Error("Image illisible."), "L’image n’a pas pu être enregistrée."), "Image illisible.");
});

test("a generated image and attached images share the grid, while documents and voice stay outside it", () => {
  const generated = {
    id: "dessin",
    role: "assistant",
    body: "Voici le phare.\n\n![Phare généré](/api/attachments/gen.png)",
    attachments: [image("d", "Photo jointe")],
  };
  const html = markup(generated, { markdown: true, align: "start" });
  assert.match(html, /data-count="2"/);
  assert.match(html, /Photo jointe/);
  assert.match(html, /Phare généré/);
  assert.equal(gallery.proseWithoutGallery(generated.body, gallery.galleryImages(generated, { markdown: true })), "Voici le phare.");

  const mixed = {
    id: "mix",
    attachments: [
      image("e", "Seul"),
      { id: "document_1", kind: "document", name: "notes.pdf", byte_length: 20, page_label: "1 page" },
      { id: "voice_1", kind: "voice", duration_ms: 1000, transcript: "" },
    ],
  };
  const mixedHtml = markup(mixed);
  assert.match(mixedHtml, /data-count="1"/);
  assert.match(mixedHtml, /gb-document-chip/);
  assert.match(mixedHtml, /Mémo vocal/);
  assert.equal((mixedHtml.match(/class="gb-image-cell"/g) || []).length, 1);
});

test("removing gallery images leaves every other character of the reply untouched", () => {
  const strip = (body) => gallery.proseWithoutGallery(body, gallery.galleryImages({ body }, { markdown: true }));
  const hardBreak = "first line  \nsecond line";
  assert.equal(strip(hardBreak), hardBreak);
  const fenced = "```\nun\n\n\n\ndeux\n```";
  assert.equal(strip(fenced), fenced);
  assert.equal(strip("avant ![x](/api/attachments/abc) après  \nsuite"), "avant  après  \nsuite");
  assert.equal(strip("Voici.  \n\n\n\n![x](/api/attachments/abc)\n"), "Voici.");
  assert.equal(strip("![x](/api/attachments/abc)\n\nTexte"), "Texte");
});

test("an image written inside code stays text and never becomes a thumbnail", () => {
  const sample = "![x](/api/attachments/abc)";
  const images = (body) => gallery.galleryImages({ body }, { markdown: true });
  const fenced = `Exemple :\n\n\`\`\`md\n${sample}\n\`\`\`\n`;
  assert.equal(images(fenced).length, 0);
  assert.equal(gallery.proseWithoutGallery(fenced, images(fenced)), fenced);
  const tilde = `~~~\n${sample}\n~~~`;
  assert.equal(images(tilde).length, 0);
  const unclosed = `\`\`\`\n${sample}`;
  assert.equal(images(unclosed).length, 0, "a fence still streaming is code to the end");
  const inline = `Écris \`${sample}\` pour afficher une image.`;
  assert.equal(images(inline).length, 0);
  assert.equal(gallery.proseWithoutGallery(inline, images(inline)), inline);
  const double = `Écris \`\`a \` b ${sample}\`\` ici.`;
  assert.equal(images(double).length, 0);
  const mixed = `${sample}\n\n\`\`\`\n${sample}\n\`\`\`\n\nEt \`${sample}\``;
  assert.equal(images(mixed).length, 1);
  assert.equal(gallery.proseWithoutGallery(mixed, images(mixed)), `\`\`\`\n${sample}\n\`\`\`\n\nEt \`${sample}\``);
  const outside = "Un `x` puis ![y](/api/attachments/def)";
  assert.equal(images(outside).length, 1);
  const html = markup({ id: "code", role: "assistant", body: fenced }, { markdown: true });
  assert.doesNotMatch(html, /gb-image-cell/);
});

test("the real Content-Type decides the type and the extension of an image without an extension", async () => {
  const [target] = gallery.galleryImages({ attachments: [{ id: "attachment_x", name: "Ciel violet", caption: "Ciel violet" }] });
  const fetchAs = (type) => async () => new Response(PNG, { status: 200, headers: type ? { "content-type": type } : {} });
  const posted = [];
  const native = { async postMessage(body) { posted.push(body); return { ok: true, name: body.name }; } };
  assert.equal(await gallery.saveGalleryImage(target, { fetch: fetchAs("image/jpeg; charset=binary"), native }), "Ciel violet.jpg");
  assert.equal(posted[0].mime, "image/jpeg");
  assert.equal(posted[0].name, "Ciel violet.jpg");
  await gallery.saveGalleryImage(target, { fetch: fetchAs(null), native });
  assert.equal(posted[1].mime, "image/png", "the default only fills in when the response says nothing");
  assert.equal(posted[1].name, "Ciel violet.png");
  await gallery.saveGalleryImage(target, { fetch: fetchAs("text/html"), native });
  assert.equal(posted[2].mime, "image/png");
  const named = gallery.galleryImages({ attachments: [{ id: "attachment_y", name: "photo.png", mime_type: "image/png" }] })[0];
  await gallery.saveGalleryImage(named, { fetch: fetchAs("image/jpeg"), native });
  assert.equal(posted[3].mime, "image/jpeg");
  assert.equal(posted[3].name, "photo.jpg", "the extension follows the type");
  assert.equal(gallery.imageFilename({ name: "photo.jpeg" }, "image/jpeg"), "photo.jpeg");
  const fromMarkdown = gallery.galleryImages({ body: "![x](/api/attachments/abc)" }, { markdown: true })[0];
  await gallery.saveGalleryImage(fromMarkdown, { fetch: fetchAs("image/webp"), native });
  assert.equal(posted[4].name, "abc.webp");
});

test("the browser copy hands the clipboard a promise of the image inside the user gesture", async () => {
  const [target] = gallery.galleryImages({ attachments: [image("a", "Phare")] });
  let release;
  const gate = new Promise((resolve) => { release = resolve; });
  const order = [];
  const written = [];
  const fetchLate = async () => { order.push("fetch"); await gate; order.push("fetched"); return new Response(PNG, { status: 200, headers: { "content-type": "image/png" } }); };
  const clipboard = { write(items) { order.push("write"); written.push(...items); return Promise.resolve(); } };
  const previous = globalThis.ClipboardItem;
  globalThis.ClipboardItem = class { constructor(parts) { this.parts = parts; } };
  try {
    const copying = gallery.copyGalleryImage(target, { fetch: fetchLate, clipboard });
    assert.deepEqual(order, ["fetch", "write"], "write is called synchronously, before the fetch resolves");
    assert.ok(written[0].parts["image/png"] instanceof Promise, "the item receives a promise of a Blob");
    release();
    const result = await copying;
    assert.equal(result.type, "image/png");
    const blob = await written[0].parts["image/png"];
    assert.deepEqual(new Uint8Array(await blob.arrayBuffer()), PNG);
  } finally {
    globalThis.ClipboardItem = previous;
  }
  const failing = gallery.copyGalleryImage(target, {
    fetch: async () => new Response("", { status: 404 }),
    clipboard: { async write([item]) { await Object.values(item)[0]; } },
  });
  await assert.rejects(failing, (error) => gallery.galleryFailureText(error, "autre") === "Image introuvable.");
});

test("copying a JPEG, GIF, WebP or BMP in the browser writes a PNG drawn on a canvas", async () => {
  const [target] = gallery.galleryImages({ attachments: [{ id: "attachment_j", name: "Ciel violet", caption: "Ciel violet" }] });
  const JPEG = Uint8Array.from([0xff, 0xd8, 0xff, 0xe0, 1, 2, 3]);
  const drawn = [];
  const decoded = [];
  const io = (type) => ({
    fetch: async () => new Response(JPEG, { status: 200, headers: { "content-type": type } }),
    async decode(blob) { decoded.push(blob.type); return { width: 3, height: 2, close() { drawn.push("close"); } }; },
    document: {
      createElement(tag) {
        assert.equal(tag, "canvas");
        return {
          width: 0,
          height: 0,
          getContext(kind) { assert.equal(kind, "2d"); return { drawImage(bitmap, x, y) { drawn.push([bitmap.width, bitmap.height, x, y]); } }; },
          toBlob(done, mime) { assert.equal(mime, "image/png"); done(new Blob([PNG], { type: "image/png" })); },
        };
      },
    },
  });
  for (const type of ["image/jpeg", "image/gif", "image/webp", "image/bmp"]) {
    const written = [];
    const result = await gallery.copyGalleryImage(target, { ...io(type), clipboard: { async write(items) { written.push(...items); } } });
    assert.equal(result.type, "image/png", type);
    const blob = await (written[0].getType ? written[0].getType("image/png") : written[0]["image/png"]);
    assert.equal(blob.type, "image/png");
    assert.deepEqual(new Uint8Array(await blob.arrayBuffer()), PNG, `${type} is converted, not passed through`);
  }
  assert.deepEqual(decoded, ["image/jpeg", "image/gif", "image/webp", "image/bmp"]);
  assert.deepEqual(drawn.slice(0, 2), [[3, 2, 0, 0], "close"]);

  const unreadable = {
    ...io("image/jpeg"),
    async decode() { throw new DOMException("The source image cannot be decoded.", "InvalidStateError"); },
    clipboard: { async write([item]) { await (item.getType ? item.getType("image/png") : item["image/png"]); } },
  };
  await assert.rejects(gallery.copyGalleryImage(target, unreadable), (error) => {
    assert.equal(gallery.galleryFailureText(error, "autre"), "Image illisible.");
    return true;
  });
});

test("a fence opened on a list item closes on its own indented fence and spares the image after it", () => {
  const sample = "![x](/api/attachments/abc)";
  const images = (body) => gallery.galleryImages({ body }, { markdown: true });
  const result = "![result](/api/generated/result.png)";
  const listed = `- \`\`\`text\n  ${sample}\n  \`\`\`\n\n${result}`;
  assert.deepEqual(images(listed).map((entry) => entry.url), ["/api/generated/result.png"]);
  assert.equal(gallery.proseWithoutGallery(listed, images(listed)), `- \`\`\`text\n  ${sample}\n  \`\`\``);
  const numbered = `1. \`\`\`\n   ${sample}\n   \`\`\`\n2. suite ${result}`;
  assert.deepEqual(images(numbered).map((entry) => entry.url), ["/api/generated/result.png"]);
  const nested = `- a\n  - ~~~\n    ${sample}\n    ~~~\n\n${result}`;
  assert.deepEqual(images(nested).map((entry) => entry.url), ["/api/generated/result.png"]);
  const quoted = `> \`\`\`\n> ${sample}\n> \`\`\`\n\n${result}`;
  assert.deepEqual(images(quoted).map((entry) => entry.url), ["/api/generated/result.png"]);
  assert.equal(images(`- \`\`\`\n  ${sample}`).length, 0, "a list fence still streaming is code to the end");
});

const memoMessage = (words, extra = {}) => ({
  id: "memo-msg",
  body: "Écoute ceci.",
  attachments: [{ id: "voice_1", kind: "voice", duration_ms: 20000, transcript: "Premier point le budget", language: "fr", words, ...extra }],
});
const spoken = [{ text: "Premier", start_ms: 0 }, { text: "point", start_ms: 600 }, { text: "le", start_ms: null }, { text: "budget", start_ms: 1200 }];

test("a voice memo shows a speed control starting at 1, or at the speed of its bot", () => {
  const plain = markup(memoMessage(spoken));
  assert.match(plain, /aria-label="Vitesse de lecture du mémo, de 0,5 à 2"[^>]*value="1"|value="1"[^>]*aria-label="Vitesse de lecture du mémo, de 0,5 à 2"/);
  const bot = markup(memoMessage(spoken), { memoSpeed: 1.5 });
  assert.match(bot, /value="1,5"/);
  assert.match(markup(memoMessage(spoken), { memoSpeed: 3 }), /value="1"/, "a speed out of range is not shown");
});

test("every word with a start is a button that says where it jumps, and a word without one is not", () => {
  const html = markup(memoMessage(spoken));
  assert.equal((html.match(/<button[^>]*class="gb-memo-word"/g) || []).length, 3);
  assert.match(html, /data-start-ms="600"/);
  assert.match(html, /<span class="gb-memo-word is-untimed"[^>]*>le<\/span>/);
  assert.doesNotMatch(html, /<button[^>]*>le<\/button>/);
});

test("a memo kept before word starts existed still shows its transcript as text", () => {
  const html = markup(memoMessage([]));
  assert.match(html, /<summary>Transcription<\/summary><p>Premier point le budget<\/p>/);
  assert.doesNotMatch(html, /gb-memo-word/);
});

test("the memo words stay readable and clickable in the stylesheet", () => {
  const css = readFileSync(new URL("./chat.css", import.meta.url), "utf8");
  assert.match(css, /\.gb-memo-word\s*\{[^}]*cursor:\s*pointer/);
  assert.match(css, /\.gb-memo-word:focus-visible/);
  assert.match(css, /\.gb-memo-speed input\s*\{/);
});

test("the shell hands the thread the speed of the bot of the open conversation", () => {
  const shell = readFileSync(new URL("../../ui/app.js", import.meta.url), "utf8");
  assert.match(shell, /memoSpeed: state\.voice\?\.bots\?\.\[conversation\?\.bot_id\]\?\.speed/);
});
