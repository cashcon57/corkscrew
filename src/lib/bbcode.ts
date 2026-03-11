/**
 * Simple BBCode to HTML converter for NexusMods descriptions.
 *
 * Handles the BBCode tags commonly used on NexusMods:
 * [b], [i], [u], [s], [url], [img], [color], [size], [center],
 * [quote], [code], [list], [*], [spoiler], [line], [font]
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

  // [color=...]...[/color]
  html = html.replace(
    /\[color=([^\]]+)\]([\s\S]*?)\[\/color\]/gi,
    (_m, color: string, text: string) => {
      const safeColor = color.replace(/[^a-zA-Z0-9#,() .%-]/g, "");
      return `<span style="color:${safeColor}">${text}</span>`;
    },
  );

  // [size=...]...[/size] — NexusMods uses numeric sizes (1-7)
  html = html.replace(
    /\[size=([^\]]+)\]([\s\S]*?)\[\/size\]/gi,
    (_m, size: string, text: string) => {
      const sizeMap: Record<string, string> = {
        "1": "0.7em", "2": "0.85em", "3": "1em", "4": "1.2em",
        "5": "1.5em", "6": "2em", "7": "2.5em",
      };
      const css = sizeMap[size.trim()] || `${size.trim()}px`;
      return `<span style="font-size:${css}">${text}</span>`;
    },
  );

  // [font=...]...[/font]
  html = html.replace(
    /\[font=([^\]]+)\]([\s\S]*?)\[\/font\]/gi,
    (_m, _font: string, text: string) => text, // Strip font tags — don't apply custom fonts
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

  // [youtube]...[/youtube]
  html = html.replace(
    /\[youtube\]([\s\S]*?)\[\/youtube\]/gi,
    (_m, id: string) => {
      const safeId = id.trim().replace(/[^a-zA-Z0-9_-]/g, "");
      return `<iframe width="560" height="315" src="https://www.youtube-nocookie.com/embed/${safeId}" frameborder="0" allowfullscreen style="max-width:100%"></iframe>`;
    },
  );

  // Convert remaining newlines to <br>
  html = html.replace(/\n/g, "<br>");

  return html;
}
