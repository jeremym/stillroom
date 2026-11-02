import type { AnchorHTMLAttributes, ImgHTMLAttributes, MouseEvent } from "react";
import Markdown, { type Components } from "react-markdown";
import { api } from "../api";

/* Markdown written by a model is shown, not trusted. An image is loaded only from the app's own
   sources (an inline data: or blob: URL, or a file served by Tauri's asset protocol); any other
   source is shown as text, so rendering a result never reaches out to a remote host. A link shows
   its address beside the text and is opened by the Rust side, in the browser, only when clicked. */

/** Sources an image may be loaded from. Everything else is a remote host, or a scheme we do not know. */
export function imageAllowed(src: string | undefined): boolean {
  if (!src) return false;
  const s = src.trim().toLowerCase();
  return s.startsWith("data:") || s.startsWith("blob:") || s.startsWith("asset:") || s.startsWith("http://asset.localhost/") || s.startsWith("https://asset.localhost/");
}

/** Links the Rust side will open: web and mail addresses. Rust checks again. */
export function linkOpenable(href: string | undefined): boolean {
  if (!href) return false;
  const h = href.trim().toLowerCase();
  return h.startsWith("http://") || h.startsWith("https://") || h.startsWith("mailto:");
}

function Img({ src, alt }: ImgHTMLAttributes<HTMLImageElement>) {
  const source = typeof src === "string" ? src : undefined;
  if (imageAllowed(source)) return <img src={source} alt={alt ?? ""} />;
  return <span className="muted mono">[image: {source ?? "no source"}{alt ? ` · ${alt}` : ""}]</span>;
}

function Link({ href, children }: AnchorHTMLAttributes<HTMLAnchorElement>) {
  if (!linkOpenable(href)) {
    return <span>{children}{href ? <span className="muted mono"> ({href})</span> : null}</span>;
  }
  const open = (e: MouseEvent) => {
    e.preventDefault();
    void api.openLink(href!).catch(() => {});
  };
  return (
    <>
      <a href={href} onClick={open} rel="noreferrer">{children}</a>
      <span className="muted mono"> ({href})</span>
    </>
  );
}

const components: Components = { img: Img, a: Link };

/** The URL is kept as written so the image and link components can show it; they decide what loads. */
const keepUrl = (url: string) => url;

export function SafeMarkdown({ children }: { children: string }) {
  return <Markdown components={components} urlTransform={keepUrl}>{children}</Markdown>;
}
