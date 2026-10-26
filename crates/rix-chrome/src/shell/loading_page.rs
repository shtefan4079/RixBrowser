use rix_core::url::RixUrl;

const PAGE_STYLE: &str = r##"
  :root {
    --bg-top: #12161f;
    --bg-bottom: #0b0e14;
    --card: #151a23;
    --card-raised: #1b212b;
    --border: rgba(255, 255, 255, 0.08);
    --border-strong: rgba(255, 255, 255, 0.16);
    --text: #e9edf5;
    --muted: #97a1b4;
    --quiet: #6f7889;
    --accent: #7c9cff;
    --accent-deep: #5b7cf0;
    --accent-soft: rgba(124, 156, 255, 0.16);
    --accent-shadow: rgba(63, 95, 215, 0.30);
    --danger: #ef7d84;
    --danger-soft: rgba(239, 125, 132, 0.12);
    --danger-border: rgba(239, 125, 132, 0.28);
    --track: #272e3a;
    --button-text: #ffffff;
    color-scheme: dark;
  }
  html[data-theme="light"] {
    --bg-top: #f7f9fc;
    --bg-bottom: #e7ecf4;
    --card: #ffffff;
    --card-raised: #f5f7fb;
    --border: #e0e5ee;
    --border-strong: #c9d2df;
    --text: #171b23;
    --muted: #5d6675;
    --quiet: #7c8595;
    --accent: #3f5fd7;
    --accent-deep: #3350c4;
    --accent-soft: rgba(63, 95, 215, 0.12);
    --accent-shadow: rgba(63, 95, 215, 0.22);
    --danger: #c4464f;
    --danger-soft: rgba(196, 70, 79, 0.09);
    --danger-border: rgba(196, 70, 79, 0.24);
    --track: #e5eaf2;
    --button-text: #ffffff;
    color-scheme: light;
  }
  * { box-sizing: border-box; }
  html, body {
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    margin: 0;
    padding: 0;
  }
  body {
    overflow: hidden;
    background-color: var(--bg-bottom);
    background-image: linear-gradient(170deg, var(--bg-top) 0%, var(--bg-bottom) 72%);
    color: var(--text);
    font-family: system-ui, "Noto Sans", "Liberation Sans", "DejaVu Sans", Arial, sans-serif;
    font-size: 15px;
    line-height: 1.5;
  }
  .page {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 14px;
    width: 100%;
    max-width: 760px;
    height: 100%;
    min-width: 0;
    min-height: 100%;
    margin: 0 auto;
    padding: 28px 20px;
    overflow: hidden;
    text-align: center;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 18px;
    width: 100%;
    max-width: 520px;
    min-width: 0;
    padding: 28px;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: 16px;
    background-color: var(--card);
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.34);
    text-align: left;
  }
  html[data-theme="light"] .card {
    box-shadow: 0 20px 52px rgba(23, 35, 60, 0.10);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .mark, .icon {
    display: flex;
    flex: 0 0 38px;
    align-items: center;
    justify-content: center;
    width: 38px;
    height: 38px;
    border-radius: 12px;
  }
  .mark {
    background-image: linear-gradient(150deg, var(--accent) 0%, var(--accent-deep) 100%);
    box-shadow: 0 10px 22px var(--accent-shadow);
    color: #ffffff;
  }
  .mark svg { width: 22px; height: 22px; }
  .icon {
    background-color: var(--danger-soft);
    color: var(--danger);
  }
  .icon svg { width: 22px; height: 22px; }
  .brand-name {
    min-width: 0;
    overflow: hidden;
    color: var(--muted);
    font-size: 13px;
    font-weight: 700;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .copy {
    display: flex;
    flex-direction: column;
    gap: 7px;
    min-width: 0;
  }
  h1 {
    margin: 0;
    font-size: 27px;
    font-weight: 700;
    line-height: 1.2;
  }
  .host { color: var(--accent); }
  .url {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    color: var(--muted);
    font-family: "DejaVu Sans Mono", "Liberation Mono", ui-monospace, monospace;
    font-size: 12px;
    line-height: 1.45;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .steps {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) minmax(0, 1fr) minmax(0, 1fr);
    gap: 8px;
    width: 100%;
    min-width: 0;
  }
  .step {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .step-bar {
    width: 100%;
    height: 4px;
    border-radius: 2px;
    background-color: var(--track);
  }
  .step-label {
    overflow: hidden;
    color: var(--quiet);
    font-size: 11px;
    line-height: 1.2;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .step.active .step-bar { background-color: var(--accent); }
  .step.active .step-label {
    color: var(--text);
  }
  .waiting {
    margin: 0;
    color: var(--muted);
    font-size: 13px;
    line-height: 1.45;
  }
  .detail {
    min-height: 52px;
    max-height: 96px;
    padding: 12px 14px;
    overflow: hidden;
    border: 1px solid var(--danger-border);
    border-radius: 12px;
    background-color: var(--danger-soft);
  }
  .detail p {
    margin: 0;
    color: var(--text);
    font-size: 13px;
    line-height: 1.5;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .button {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 40px;
    padding: 0 18px;
    border: 1px solid transparent;
    border-radius: 11px;
    font-size: 13px;
    font-weight: 700;
    line-height: 1;
    text-decoration: none;
    white-space: nowrap;
  }
  .button.primary {
    background-color: var(--accent);
    color: var(--button-text);
  }
  .button.primary:hover { background-color: var(--accent-deep); }
  .button.primary:active { opacity: 0.88; }
  .button.secondary {
    border-color: var(--border);
    background-color: transparent;
    color: var(--text);
  }
  .button.secondary:hover {
    border-color: var(--border-strong);
    background-color: var(--card-raised);
  }
  .button.secondary:active { opacity: 0.88; }
  .button:focus-visible {
    border-color: var(--accent);
  }
  .footer {
    margin: 0;
    color: var(--quiet);
    font-size: 11px;
    line-height: 1.4;
    text-align: center;
  }
  @media (max-width: 520px) {
    .page { padding: 22px 16px; gap: 12px; }
    .card { gap: 16px; padding: 22px; border-radius: 14px; }
    h1 { font-size: 23px; }
    .mark, .icon { flex-basis: 36px; width: 36px; height: 36px; border-radius: 11px; }
    .mark svg, .icon svg { width: 21px; height: 21px; }
  }
  @media (max-height: 420px) {
    .page { padding: 14px 16px; gap: 10px; }
    .card { gap: 12px; padding: 18px 20px; }
    .brand { gap: 8px; }
    .mark, .icon { flex-basis: 32px; width: 32px; height: 32px; border-radius: 10px; }
    .mark svg, .icon svg { width: 19px; height: 19px; }
    h1 { font-size: 20px; }
    .copy { gap: 5px; }
    .url { font-size: 11px; }
    .steps { gap: 6px; }
    .step { gap: 5px; }
    .waiting { font-size: 12px; }
    .detail { min-height: 40px; max-height: 68px; padding: 8px 10px; }
    .detail p { font-size: 12px; line-height: 1.4; }
    .actions { gap: 8px; }
    .button { min-height: 34px; padding: 0 14px; font-size: 12px; }
    .footer { font-size: 10px; }
  }
"##;

const RIX_MARK: &str = r##"<svg viewBox="0 0 32 32" aria-hidden="true"><path fill="currentColor" d="M6 4h11a9 9 0 0 1 4.6 16.73L27.4 28h-6.1L16.6 20.6H12V28H6V4Zm6 6v4.6h5a2.3 2.3 0 1 0 0-4.6h-5Z" /></svg>"##;

const ALERT_ICON: &str = r##"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="M12 3 2.7 20h18.6L12 3Z" /><path d="M12 9v5" /><path d="M12 17.3h0.01" /></svg>"##;

fn theme_name(theme: &str) -> &str {
    if theme.eq_ignore_ascii_case("light") {
        "light"
    } else {
        "dark"
    }
}

fn display_host(url: &str) -> String {
    let trimmed = url.trim();
    if let Ok(parsed) = RixUrl::parse(trimmed) {
        if let Some(host) = parsed.host_str().filter(|host| !host.is_empty()) {
            return host.to_string();
        }
    }
    if trimmed.is_empty() {
        "this page".to_string()
    } else {
        trimmed.to_string()
    }
}

fn display_url(url: &str) -> &str {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        "this page"
    } else {
        trimmed
    }
}

pub(super) fn loading_page_html(url: &str, theme: &str) -> String {
    let host = crate::content::html_escape(&display_host(url));
    let url = crate::content::html_escape(display_url(url));
    let theme = crate::content::html_escape(theme_name(theme));
    format!(
        r##"<!DOCTYPE html>
<html lang="en" data-theme="{theme}">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0" />
<title>Loading {host}</title>
<style>
{style}
</style>
</head>
<body>
  <div class="page">
    <div class="card" role="status" aria-live="polite">
      <div class="brand">
        <span class="mark" aria-hidden="true">{mark}</span>
        <span class="brand-name">RixBrowser</span>
      </div>
      <div class="copy">
        <h1>Opening <span class="host">{host}</span></h1>
        <p class="url">{url}</p>
      </div>
      <div class="steps" role="img" aria-label="Loading stages: request, styles, scripts, ready">
        <div class="step active"><span class="step-bar"></span><span class="step-label">Request</span></div>
        <div class="step"><span class="step-bar"></span><span class="step-label">Styles</span></div>
        <div class="step"><span class="step-bar"></span><span class="step-label">Scripts</span></div>
        <div class="step"><span class="step-bar"></span><span class="step-label">Ready</span></div>
      </div>
      <p class="waiting">Waiting for the site to respond</p>
    </div>
    <p class="footer">RixBrowser · 0.1.0-alpha.1</p>
  </div>
</body>
</html>
"##,
        theme = theme,
        style = PAGE_STYLE,
        mark = RIX_MARK,
        host = host,
        url = url,
    )
}

pub(super) fn error_page_html(url: &str, error: &str, theme: &str) -> String {
    let retry_url = crate::content::html_escape(url.trim());
    let host = crate::content::html_escape(&display_host(url));
    let url = crate::content::html_escape(display_url(url));
    let detail = if error.trim().is_empty() {
        "The page could not be loaded."
    } else {
        error.trim()
    };
    let detail = crate::content::html_escape(detail);
    let theme = crate::content::html_escape(theme_name(theme));
    format!(
        r##"<!DOCTYPE html>
<html lang="en" data-theme="{theme}">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0" />
<title>Couldn't open {host}</title>
<style>
{style}
</style>
</head>
<body>
  <div class="page">
    <div class="card">
      <div class="brand">
        <span class="icon" aria-hidden="true">{icon}</span>
        <span class="brand-name">RixBrowser</span>
      </div>
      <div class="copy">
        <h1>Couldn't open <span class="host">{host}</span></h1>
        <p class="url">{url}</p>
      </div>
      <div class="detail" role="alert"><p>{detail}</p></div>
      <div class="actions">
        <a class="button primary" href="{retry_url}">Try again</a>
        <a class="button secondary" href="https://www.google.com">Back to start</a>
      </div>
    </div>
    <p class="footer">RixBrowser · 0.1.0-alpha.1</p>
  </div>
</body>
</html>
"##,
        theme = theme,
        style = PAGE_STYLE,
        icon = ALERT_ICON,
        host = host,
        url = url,
        retry_url = retry_url,
        detail = detail,
    )
}
