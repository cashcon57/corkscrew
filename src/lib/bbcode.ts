/**
 * Simple BBCode to HTML converter for NexusMods descriptions.
 *
 * Handles the BBCode tags commonly used on NexusMods:
 * [b], [i], [u], [s], [url], [img], [center], [left], [right],
 * [quote], [code], [list], [*], [spoiler], [line], [hr],
 * [heading], [h1], [h2], [h3], [youtube]
 *
 * Deliberately STRIPPED (tag removed, content kept):
 * - [color]  — dark-themed app picks its own text color; verbatim hex
 *              colors from authors caused dark-on-dark invisible text
 * - [size]   — large size tags from NM authors can break layout
 * - [font]   — custom font families don't render well cross-platform
 *
 * Unknown tags are stripped (open + close) at the end so author-specific
 * BBCode (e.g. [acronym], [b1], etc.) doesn't bleed through raw.
 */
export function bbcodeToHtml(input: string): string {
  if (!input) return "";

  let html = input;

  // Preserve raw HTML line breaks before escaping (NexusMods mixes HTML <br> with BBCode)
  html = html.replace(/<br\s*\/?>/gi, "\n");

  // Escape HTML entities first (prevent XSS from raw HTML in BBCode)
  html = html
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");

  // Line breaks
  html = html.replace(/\r\n/g, "\n");

  // [b]...[/b]
  html = html.replace(/\[b\]([\s\S]*?)\[\/b\]/gi, "<strong>$1</strong>");

  // [i]...[/i]
  html = html.replace(/\[i\]([\s\S]*?)\[\/i\]/gi, "<em>$1</em>");

  // [u]...[/u]
  html = html.replace(/\[u\]([\s\S]*?)\[\/u\]/gi, "<u>$1</u>");

  // [s]...[/s]
  html = html.replace(/\[s\]([\s\S]*?)\[\/s\]/gi, "<s>$1</s>");

  // [url=...]...[/url]
  html = html.replace(
    /\[url=([^\]]+)\]([\s\S]*?)\[\/url\]/gi,
    (_m, url: string, text: string) => {
      const safeUrl = url.replace(/"/g, "&quot;");
      return `<a href="${safeUrl}" target="_blank" rel="noopener noreferrer">${text}</a>`;
    },
  );

  // [url]...[/url]
  html = html.replace(
    /\[url\]([\s\S]*?)\[\/url\]/gi,
    (_m, url: string) => {
      const safeUrl = url.replace(/"/g, "&quot;");
      return `<a href="${safeUrl}" target="_blank" rel="noopener noreferrer">${safeUrl}</a>`;
    },
  );

  // [img]...[/img]
  html = html.replace(
    /\[img\]([\s\S]*?)\[\/img\]/gi,
    (_m, url: string) => {
      const safeUrl = url.trim().replace(/"/g, "&quot;");
      return `<img src="${safeUrl}" alt="" style="max-width:100%;height:auto;" loading="lazy" />`;
    },
  );

  // [color=...]...[/color] — STRIP entirely (keep content).
  // Author-specified colors (esp. #000000) caused dark-on-dark invisible text
  // in our dark theme. Let the app's CSS pick the text color.
  html = html.replace(
    /\[color=[^\]]*\]([\s\S]*?)\[\/color\]/gi,
    "$1",
  );

  // [size=...]...[/size] — STRIP entirely (keep content).
  // Large author-specified sizes broke layout.
  html = html.replace(
    /\[size=[^\]]*\]([\s\S]*?)\[\/size\]/gi,
    "$1",
  );

  // [font=...]...[/font] — strip, keep content (cross-platform font families)
  html = html.replace(
    /\[font=[^\]]*\]([\s\S]*?)\[\/font\]/gi,
    "$1",
  );

  // [left]...[/left]
  html = html.replace(
    /\[left\]([\s\S]*?)\[\/left\]/gi,
    '<div style="text-align:left">$1</div>',
  );

  // [center]...[/center]
  html = html.replace(
    /\[center\]([\s\S]*?)\[\/center\]/gi,
    '<div style="text-align:center">$1</div>',
  );

  // [right]...[/right]
  html = html.replace(
    /\[right\]([\s\S]*?)\[\/right\]/gi,
    '<div style="text-align:right">$1</div>',
  );

  // [quote]...[/quote] and [quote=author]...[/quote]
  html = html.replace(
    /\[quote(?:=[^\]]*)?\]([\s\S]*?)\[\/quote\]/gi,
    '<blockquote style="border-left:3px solid var(--separator);padding-left:12px;margin:8px 0;color:var(--text-secondary)">$1</blockquote>',
  );

  // [code]...[/code]
  html = html.replace(
    /\[code\]([\s\S]*?)\[\/code\]/gi,
    '<pre style="background:var(--bg-tertiary);padding:8px;border-radius:4px;overflow-x:auto"><code>$1</code></pre>',
  );

  // [spoiler]...[/spoiler]
  html = html.replace(
    /\[spoiler\]([\s\S]*?)\[\/spoiler\]/gi,
    '<details><summary>Spoiler</summary>$1</details>',
  );

  // [line] / [hr]
  html = html.replace(/\[line\]/gi, '<hr style="border:none;border-top:1px solid var(--separator);margin:12px 0" />');
  html = html.replace(/\[hr\]/gi, '<hr style="border:none;border-top:1px solid var(--separator);margin:12px 0" />');

  // [list] with [*] items
  html = html.replace(
    /\[list\]([\s\S]*?)\[\/list\]/gi,
    (_m, content: string) => {
      const items = content
        .split(/\[\*\]/)
        .filter((s) => s.trim())
        .map((s) => `<li>${s.trim()}</li>`)
        .join("");
      return `<ul style="margin:4px 0;padding-left:20px">${items}</ul>`;
    },
  );

  // [list=1] ordered list
  html = html.replace(
    /\[list=\d+\]([\s\S]*?)\[\/list\]/gi,
    (_m, content: string) => {
      const items = content
        .split(/\[\*\]/)
        .filter((s) => s.trim())
        .map((s) => `<li>${s.trim()}</li>`)
        .join("");
      return `<ol style="margin:4px 0;padding-left:20px">${items}</ol>`;
    },
  );

  // [heading]...[/heading]
  html = html.replace(
    /\[heading\]([\s\S]*?)\[\/heading\]/gi,
    '<h3 style="margin:12px 0 4px">$1</h3>',
  );

  // [h1]/[h2]/[h3]...[/hN]
  html = html.replace(
    /\[h1\]([\s\S]*?)\[\/h1\]/gi,
    '<h1 style="margin:14px 0 6px;font-size:1.4em">$1</h1>',
  );
  html = html.replace(
    /\[h2\]([\s\S]*?)\[\/h2\]/gi,
    '<h2 style="margin:12px 0 5px;font-size:1.25em">$1</h2>',
  );
  html = html.replace(
    /\[h3\]([\s\S]*?)\[\/h3\]/gi,
    '<h3 style="margin:12px 0 4px;font-size:1.1em">$1</h3>',
  );

  // Stray [*] outside of [list] — convert to a bullet
  html = html.replace(/\[\*\]/g, "• ");

  // [youtube]...[/youtube]
  html = html.replace(
    /\[youtube\]([\s\S]*?)\[\/youtube\]/gi,
    (_m, id: string) => {
      const safeId = id.trim().replace(/[^a-zA-Z0-9_-]/g, "");
      return `<iframe width="560" height="315" src="https://www.youtube-nocookie.com/embed/${safeId}" frameborder="0" allowfullscreen style="max-width:100%"></iframe>`;
    },
  );

  // Catch-all: strip any remaining unknown BBCode tags so they don't render raw.
  // Runs LAST so all specific tags above have already been processed.
  // Matches:
  //   [tag]           — opening tags
  //   [tag=anything]  — opening tags with args
  //   [/tag]          — closing tags
  html = html.replace(/\[\/?[a-zA-Z][a-zA-Z0-9]*(?:=[^\]]*)?\]/g, "");

  // Convert remaining newlines to <br>
  html = html.replace(/\n/g, "<br>");

  return html;
}
