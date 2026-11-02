import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { SafeMarkdown, imageAllowed, linkOpenable } from "./markdown";

describe("SafeMarkdown", () => {
  it("shows a remote image as text instead of loading it", () => {
    const html = renderToStaticMarkup(<SafeMarkdown>{"Look: ![a cat](https://example.com/cat.png)"}</SafeMarkdown>);
    expect(html).not.toContain("<img");
    expect(html).toContain("[image: https://example.com/cat.png · a cat]");
  });
  it("loads an inline or app-served image", () => {
    const html = renderToStaticMarkup(<SafeMarkdown>{"![dot](data:image/gif;base64,R0lGODlhAQABAAAAACw=)"}</SafeMarkdown>);
    expect(html).toContain("<img");
    expect(html).toContain('src="data:image/gif;base64,R0lGODlhAQABAAAAACw="');
  });
  it("shows a link with its address, and keeps unknown schemes as text", () => {
    const web = renderToStaticMarkup(<SafeMarkdown>{"[docs](https://example.com/docs)"}</SafeMarkdown>);
    expect(web).toContain('<a href="https://example.com/docs"');
    expect(web).toContain("(https://example.com/docs)");
    const odd = renderToStaticMarkup(<SafeMarkdown>{"[run](javascript:alert(1))"}</SafeMarkdown>);
    expect(odd).not.toContain("<a ");
    expect(odd).toContain("run");
  });
  it("knows which sources and links are allowed", () => {
    expect(imageAllowed("asset://localhost/Users/x/out.png")).toBe(true);
    expect(imageAllowed("http://asset.localhost/Users/x/out.png")).toBe(true);
    expect(imageAllowed("blob:tauri://localhost/abc")).toBe(true);
    expect(imageAllowed("https://example.com/x.png")).toBe(false);
    expect(imageAllowed("file:///etc/passwd")).toBe(false);
    expect(linkOpenable("mailto:someone@example.com")).toBe(true);
    expect(linkOpenable("file:///etc/passwd")).toBe(false);
  });
});
