//! Share files (DESIGN.md, The web version on GitHub Pages, *Share files*,
//! 17e): one `.html` file that holds a workbook and opens it in the web
//! version. The page is small and loads nothing from elsewhere: the
//! workbook's name, a line on what Gossia is, a button *Open in Gossia*, and
//! the `.gossia` file unchanged as base64. The button opens the site in a
//! new window, which says it is ready with `postMessage`; the page then
//! sends the workbook to it the same way, to the site's origin only.
//!
//! Gossia reads share files too: [`is_share`] tells one from other files and
//! [`read`] takes the workbook out of it. Here, rather than in the window's
//! crate, so that `bidwillii share` writes them too.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use gossia_i18n::tr;

/// The extension of a workbook.
const EXTENSION: &str = "gossia";

/// The address of the official site, fixed when Gossia is built.
/// `GOSSIA_SITE` at build time gives another (a fork's site).
pub const SITE: &str = match option_env!("GOSSIA_SITE") {
    Some(site) => site,
    None => "https://wvdveer.github.io/gossia/",
};

/// What marks a share file, near its start.
const MARK: &str = r#"<meta name="gossia-share-file" content="1">"#;
/// The meta element with the workbook's file name.
const NAME_META: &str = r#"<meta name="gossia-workbook" content=""#;
/// The element the workbook is in, as base64.
const DATA_OPEN: &str = r#"<script type="application/x-gossia-workbook" id="workbook">"#;
const DATA_CLOSE: &str = "</script>";

/// Above this many bytes, saving a share file warns that many mail systems
/// will not take it.
pub const MAIL_LIMIT: usize = 20 * 1024 * 1024;

/// The share file of the workbook `bytes`, a `.gossia` file called `name`
/// (`Orders.gossia`), with the page's texts in the interface language.
pub fn to_html(name: &str, bytes: &[u8]) -> String {
    let title = name
        .strip_suffix(&format!(".{}", EXTENSION))
        .unwrap_or(name);
    let site = SITE;
    let texts = serde_json::json!({
        "opening": tr!("Opening Gossia in a new window..."),
        "blocked": tr!(
            "The browser did not open a new window. Allow pop-ups for this page, or open {site} and drop this file on it.",
            site = site
        ),
        "sent": tr!("The workbook is open in Gossia, in the other window."),
    });
    // Nothing in a script may end it early.
    let texts = texts.to_string().replace("</", "<\\/");
    let site_js = serde_json::to_string(site)
        .unwrap_or_default()
        .replace("</", "<\\/");
    let data = STANDARD.encode(bytes);
    let lang = gossia_i18n::language().tag();
    format!(
        r#"<!doctype html>
<html lang="{lang}">
<head>
<meta charset="utf-8">
{MARK}
{NAME_META}{name_attr}">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title_text}</title>
<style>
  :root {{ color-scheme: light dark; --bg: #f6f6f6; --card: #ffffff; --text: #1e1e1e; --weak: #5c5c5c; --accent: #2f6fde; }}
  @media (prefers-color-scheme: dark) {{ :root {{ --bg: #1e1e1e; --card: #2a2a2a; --text: #e6e6e6; --weak: #a8a8a8; --accent: #5b8ff0; }} }}
  body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; background: var(--bg); color: var(--text);
         font: 16px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }}
  main {{ background: var(--card); max-width: 32rem; margin: 16px; padding: 28px; border-radius: 10px; box-shadow: 0 2px 12px #0002; }}
  h1 {{ font-size: 1.4rem; margin: 0 0 12px; overflow-wrap: anywhere; }}
  p {{ margin: 0 0 14px; }}
  .weak {{ color: var(--weak); font-size: 0.9rem; }}
  button {{ font: inherit; font-weight: 600; color: #fff; background: var(--accent); border: 0; border-radius: 6px;
           padding: 10px 20px; cursor: pointer; margin: 4px 0 14px; }}
  a {{ color: var(--accent); }}
</style>
</head>
<body>
<main>
<h1>{title_text}</h1>
<p>{holds}</p>
<p>{what}</p>
<button id="open" type="button" hidden>{open}</button>
<p id="status" role="status"></p>
<p class="weak">{fallback}</p>
</main>
{DATA_OPEN}{data}{DATA_CLOSE}
<script>
(() => {{
  const SITE = {site_js};
  const TEXTS = {texts};
  const ORIGIN = new URL(SITE).origin;
  const button = document.getElementById('open');
  const status = document.getElementById('status');
  button.hidden = false;
  button.addEventListener('click', () => {{
    const site = window.open(SITE + '#share', '_blank');
    if (!site) {{ status.textContent = TEXTS.blocked; return; }}
    status.textContent = TEXTS.opening;
    const name = document.querySelector('meta[name="gossia-workbook"]').content;
    const data = document.getElementById('workbook').textContent.trim();
    function ready(event) {{
      if (event.source !== site || event.origin !== ORIGIN || !event.data || event.data.gossia !== 'ready') return;
      window.removeEventListener('message', ready);
      const text = atob(data);
      const bytes = new Uint8Array(text.length);
      for (let i = 0; i < text.length; i++) bytes[i] = text.charCodeAt(i);
      site.postMessage({{ gossia: 'share', name, bytes }}, ORIGIN, [bytes.buffer]);
      status.textContent = TEXTS.sent;
    }}
    window.addEventListener('message', ready);
  }});
}})();
</script>
</body>
</html>
"#,
        name_attr = escape(name),
        title_text = escape(title),
        holds = escape(&tr!(
            "This file holds the Gossia workbook {name}.",
            name = title
        )),
        what = escape(tr!(
            "Gossia is a free program for structured data with formulas in Python. Its web version opens the workbook in your browser: nothing of it is sent to a server."
        )),
        open = escape(tr!("Open in Gossia")),
        fallback = fallback(site),
    )
}

/// The line for when the button does not work, with the site as a link.
fn fallback(site: &str) -> String {
    let link = format!(r#"<a href="{0}">{0}</a>"#, escape(site));
    // The place is filled after escaping, so that the link stays a link.
    escape(&tr!(
        "If the button does nothing, open {site} and drop this file on it.",
        site = "\u{1}"
    ))
    .replace('\u{1}', &link)
}

/// Whether `bytes` are a share file.
pub fn is_share(bytes: &[u8]) -> bool {
    let start = &bytes[..bytes.len().min(4096)];
    String::from_utf8_lossy(start).contains(MARK)
}

/// The workbook in a share file: its file name (`Orders.gossia`) and the
/// `.gossia` file.
pub fn read(bytes: &[u8]) -> Result<(String, Vec<u8>), String> {
    let broken = || tr!("this share file is damaged: the workbook in it cannot be read").to_owned();
    let text = std::str::from_utf8(bytes).map_err(|_| broken())?;
    if !is_share(bytes) {
        return Err(tr!("this is not a Gossia share file").to_owned());
    }
    let name = text
        .split_once(NAME_META)
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(name, _)| unescape(name))
        .ok_or_else(broken)?;
    let data = text
        .split_once(DATA_OPEN)
        .and_then(|(_, rest)| rest.split_once(DATA_CLOSE))
        .map(|(data, _)| data)
        .ok_or_else(broken)?;
    let data: String = data.chars().filter(|c| !c.is_whitespace()).collect();
    let workbook = STANDARD.decode(data).map_err(|_| broken())?;
    Ok((workbook_name(&name), workbook))
}

/// The share file's name for a workbook called `name`: `Orders.html`.
pub fn file_name(name: &str) -> String {
    let stem = name
        .strip_suffix(&format!(".{}", EXTENSION))
        .unwrap_or(name);
    format!("{stem}.html")
}

/// A name from a share file as a workbook's file name: no folders, and the
/// workbook's extension.
fn workbook_name(name: &str) -> String {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim()
        .trim_start_matches('.');
    let ext = format!(".{}", EXTENSION);
    let stem = base.strip_suffix(&ext).unwrap_or(base);
    let stem = if stem.is_empty() {
        tr!("Untitled").to_owned()
    } else {
        stem.to_owned()
    };
    format!("{stem}{ext}")
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}
