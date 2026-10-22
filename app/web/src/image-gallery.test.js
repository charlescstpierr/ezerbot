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
  const blob = typeof item.getType === "function" ? await item.getType("image/png") : item["image/png"];
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

test("the Cocoa shell opens NSSavePanel and only then writes the image", () => {
  const shell = readFileSync(new URL("../../native/main.m", import.meta.url), "utf8");
  const saveStart = shell.indexOf('![command isEqual:@"save"]');
  const saveEnd = shell.indexOf('if ([message.name isEqual:@"ezerbotVoice"])', saveStart);
  const save = shell.slice(saveStart, saveEnd);
  assert.ok(saveStart > 0 && saveEnd > saveStart, "commande save");
  const panel = save.indexOf("NSSavePanel");
  const confirmed = save.indexOf("NSModalResponseOK");
  const write = save.indexOf("writeToURL:panel.URL");
  const failure = save.indexOf("L’image n’a pas pu être enregistrée.", write);
  assert.ok(panel > 0 && panel < confirmed && confirmed < write && write < failure, save);
  assert.match(save, /cancelled/);
  assert.match(save, /\[body\[@"name"\] isKindOfClass:NSString\.class\] \? \(NSString \*\)body\[@"name"\]/);
  assert.doesNotMatch(save, /body\[@"name"\]\.lastPathComponent/);
  const policy = shell.slice(shell.indexOf("- (BOOL)isWorkspaceBlobURL:"), shell.indexOf("- (WKWebView *)webView:(WKWebView *)webView createWebViewWithConfiguration:"));
  assert.match(policy, /blob:/);
  assert.match(policy, /null\//);
  const blobBranch = policy.indexOf("workspaceBlob && action.navigationType == WKNavigationTypeLinkActivated");
  const download = policy.indexOf("WKNavigationActionPolicyDownload", blobBranch);
  const finalCancel = policy.lastIndexOf("WKNavigationActionPolicyCancel");
  assert.ok(blobBranch > 0 && download > blobBranch && finalCancel > download, policy);
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
