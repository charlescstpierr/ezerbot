import { createElement } from "react";

export function localResourceURL(url) {
  return globalThis.EzerbotLocal?.resourceURL(url) ?? url;
}

export const localMarkdownComponents = {
  img: ({ src, ...props }) => createElement("img", { ...props, src: localResourceURL(src) }),
};
