#![deny(unsafe_code)]

use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct YoutubeVideoItem {
    pub video_id: String,
    pub title: String,
    pub channel: String,
    pub views: String,
    pub published: String,
    pub duration: String,
    pub thumbnail_url: String,
}

#[derive(Debug, Clone, Default)]
pub struct NudgeInfo {
    pub title: String,
    pub subtitle: String,
}

#[derive(Debug, Clone, Default)]
pub struct WatchPageInfo {
    pub video_id: String,
    pub title: String,
    pub channel: String,
    pub views: String,
    pub subscribers: String,
    pub related_videos: Vec<YoutubeVideoItem>,
}

fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn escape_attr(input: &str) -> String {
    escape_html(input)
}

fn extract_text_from_runs(val: &Value) -> String {
    if let Some(text) = val.get("simpleText").and_then(|v| v.as_str()) {
        return text.trim().to_string();
    }
    if let Some(runs) = val.get("runs").and_then(|v| v.as_array()) {
        let mut text = String::new();
        for run in runs {
            if let Some(t) = run.get("text").and_then(|v| v.as_str()) {
                text.push_str(t);
            }
        }
        return text.trim().to_string();
    }
    String::new()
}

fn extract_thumbnail_url(val: &Value) -> String {
    if let Some(thumbs) = val.get("thumbnails").and_then(|v| v.as_array()) {
        if let Some(last) = thumbs.last() {
            if let Some(url) = last.get("url").and_then(|v| v.as_str()) {
                return url.to_string();
            }
        }
    }
    String::new()
}

fn parse_video_renderer(vr: &Value) -> Option<YoutubeVideoItem> {
    let video_id = vr.get("videoId").and_then(|v| v.as_str())?.to_string();
    if video_id.is_empty() {
        return None;
    }

    let title = vr
        .get("title")
        .map(extract_text_from_runs)
        .unwrap_or_default();

    let channel = vr
        .get("ownerText")
        .or_else(|| vr.get("longBylineText"))
        .or_else(|| vr.get("shortBylineText"))
        .map(extract_text_from_runs)
        .unwrap_or_default();

    let views = vr
        .get("viewCountText")
        .or_else(|| vr.get("shortViewCountText"))
        .map(extract_text_from_runs)
        .unwrap_or_default();

    let published = vr
        .get("publishedTimeText")
        .map(extract_text_from_runs)
        .unwrap_or_default();

    let duration = vr
        .get("lengthText")
        .map(extract_text_from_runs)
        .unwrap_or_default();

    let thumbnail_url = vr
        .get("thumbnail")
        .map(extract_thumbnail_url)
        .unwrap_or_default();

    Some(YoutubeVideoItem {
        video_id,
        title,
        channel,
        views,
        published,
        duration,
        thumbnail_url,
    })
}

fn render_search_card(item: &YoutubeVideoItem) -> String {
    let vid = escape_attr(&item.video_id);
    let title = escape_html(&item.title);
    let channel = escape_html(&item.channel);
    let views = escape_html(&item.views);
    let published = escape_html(&item.published);
    let duration = escape_html(&item.duration);
    let thumb = escape_attr(&item.thumbnail_url);

    let duration_badge = if !duration.is_empty() {
        format!(
            r#"<span style="position: absolute; bottom: 8px; right: 8px; background: rgba(0,0,0,0.8); color: #fff; padding: 2px 4px; border-radius: 4px; font-size: 12px; font-weight: 500;">{duration}</span>"#
        )
    } else {
        String::new()
    };

    let meta_sub = if !published.is_empty() && !views.is_empty() {
        format!("{views} • {published}")
    } else if !views.is_empty() {
        views
    } else {
        published
    };

    format!(
        r#"<ytd-video-renderer class="style-scope ytd-item-section-renderer" style="display: flex; margin-bottom: 16px; min-height: 140px;">
  <div id="dismissible" class="style-scope ytd-video-renderer" style="display: flex; flex-direction: row; gap: 16px; width: 100%;">
    <ytd-thumbnail class="style-scope ytd-video-renderer" style="flex-shrink: 0; width: 360px; height: 202px; border-radius: 12px; overflow: hidden; position: relative;">
      <a id="thumbnail" class="yt-simple-endpoint style-scope ytd-thumbnail" href="/watch?v={vid}" style="display: block; width: 100%; height: 100%; position: relative;">
        <img id="img" class="ytCoreImageHost ytCoreImageLoaded ytCoreImageFillParentHeight ytCoreImageFillParentWidth ytCoreImageContentModeScaleAspectFill style-scope yt-img-shadow" src="{thumb}" style="width: 100%; height: 100%; object-fit: cover; display: block; visibility: visible !important; opacity: 1 !important;" alt="{title}">
        {duration_badge}
      </a>
    </ytd-thumbnail>
    <div class="text-wrapper style-scope ytd-video-renderer" style="display: flex; flex-direction: column; flex-grow: 1; justify-content: flex-start; padding-top: 4px;">
      <div id="meta" class="style-scope ytd-video-renderer">
        <h3 class="title-and-badge style-scope ytd-video-renderer" style="margin: 0 0 6px 0; font-size: 18px; line-height: 24px; font-weight: 500;">
          <a id="video-title" class="yt-simple-endpoint style-scope ytd-video-renderer" href="/watch?v={vid}" title="{title}" style="color: var(--yt-spec-text-primary, #f1f1f1); text-decoration: none;">
            {title}
          </a>
        </h3>
        <div id="channel-info" style="color: var(--yt-spec-text-secondary, #aaa); font-size: 13px; margin-bottom: 4px;">
          <span>{channel}</span>
        </div>
        <div id="metadata-line" style="color: var(--yt-spec-text-secondary, #aaa); font-size: 12px;">
          <span>{meta_sub}</span>
        </div>
      </div>
    </div>
  </div>
</ytd-video-renderer>"#
    )
}

fn render_grid_card(item: &YoutubeVideoItem) -> String {
    let vid = escape_attr(&item.video_id);
    let title = escape_html(&item.title);
    let channel = escape_html(&item.channel);
    let views = escape_html(&item.views);
    let published = escape_html(&item.published);
    let duration = escape_html(&item.duration);
    let thumb = escape_attr(&item.thumbnail_url);

    let duration_badge = if !duration.is_empty() {
        format!(
            r#"<span style="position: absolute; bottom: 8px; right: 8px; background: rgba(0,0,0,0.8); color: #fff; padding: 2px 4px; border-radius: 4px; font-size: 12px; font-weight: 500;">{duration}</span>"#
        )
    } else {
        String::new()
    };

    let meta_sub = if !published.is_empty() && !views.is_empty() {
        format!("{views} • {published}")
    } else if !views.is_empty() {
        views
    } else {
        published
    };

    format!(
        r#"<ytd-rich-item-renderer class="style-scope ytd-rich-grid-renderer" style="width: calc(33.333% - 16px); min-width: 300px; max-width: 420px; display: flex; flex-direction: column; margin-bottom: 24px;">
  <div id="content" class="style-scope ytd-rich-item-renderer" style="width: 100%; display: flex; flex-direction: column;">
    <div id="thumbnail-container" style="width: 100%; aspect-ratio: 16/9; border-radius: 12px; overflow: hidden; position: relative;">
      <a class="yt-simple-endpoint style-scope ytd-thumbnail" href="/watch?v={vid}" style="display: block; width: 100%; height: 100%; position: relative;">
        <img class="ytCoreImageHost ytCoreImageLoaded ytCoreImageFillParentHeight ytCoreImageFillParentWidth ytCoreImageContentModeScaleAspectFill style-scope yt-img-shadow" src="{thumb}" style="width: 100%; height: 100%; object-fit: cover; display: block; visibility: visible !important; opacity: 1 !important;" alt="{title}">
        {duration_badge}
      </a>
    </div>
    <div id="details" style="display: flex; flex-direction: row; gap: 12px; margin-top: 12px;">
      <div id="meta" style="display: flex; flex-direction: column; flex-grow: 1;">
        <h3 style="margin: 0 0 4px 0; font-size: 16px; line-height: 22px; font-weight: 500; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;">
          <a id="video-title-link" href="/watch?v={vid}" title="{title}" style="color: var(--yt-spec-text-primary, #f1f1f1); text-decoration: none;">
            {title}
          </a>
        </h3>
        <div style="color: var(--yt-spec-text-secondary, #aaa); font-size: 14px; margin-bottom: 2px;">
          <span>{channel}</span>
        </div>
        <div style="color: var(--yt-spec-text-secondary, #aaa); font-size: 12px;">
          <span>{meta_sub}</span>
        </div>
      </div>
    </div>
  </div>
</ytd-rich-item-renderer>"#
    )
}

fn render_feed_nudge(nudge: &NudgeInfo) -> String {
    let title = escape_html(&nudge.title);
    let subtitle = escape_html(&nudge.subtitle);

    format!(
        r#"<div id="feed-nudge-container" style="display: flex; flex-direction: column; align-items: center; justify-content: center; padding: 60px 24px; text-align: center; max-width: 600px; margin: 40px auto; color: var(--yt-spec-text-primary, #f1f1f1);">
  <div style="width: 110px; height: 110px; border-radius: 50%; background: rgba(255,255,255,0.06); display: flex; align-items: center; justify-content: center; margin-bottom: 24px;">
    <svg viewBox="0 0 24 24" style="width: 44px; height: 44px; fill: currentColor;"><path d="M20.87,20.17l-5.59-5.59C16.35,13.35,17,11.75,17,10c0-3.87-3.13-7-7-7s-7,3.13-7,7s3.13,7,7,7c1.75,0,3.35-0.65,4.58-1.71 l5.59,5.59L20.87,20.17z M10,15c-2.76,0-5-2.24-5-5s2.24-5,5-5s5,2.24,5,5S12.76,15,10,15z"/></svg>
  </div>
  <h2 style="font-size: 24px; line-height: 32px; font-weight: 500; margin: 0 0 12px 0;">{title}</h2>
  <p style="font-size: 14px; line-height: 20px; color: var(--yt-spec-text-secondary, #aaa); margin: 0 0 24px 0;">{subtitle}</p>
</div>"#
    )
}

fn render_watch_page(watch_info: &WatchPageInfo) -> String {
    let vid = escape_attr(&watch_info.video_id);
    let title = escape_html(&watch_info.title);
    let channel = escape_html(&watch_info.channel);
    let views = escape_html(&watch_info.views);
    let subs = escape_html(&watch_info.subscribers);

    let mut related_html = String::new();
    for rel in &watch_info.related_videos {
        let r_vid = escape_attr(&rel.video_id);
        let r_title = escape_html(&rel.title);
        let r_channel = escape_html(&rel.channel);
        let r_views = escape_html(&rel.views);
        let r_duration = escape_html(&rel.duration);
        let r_thumb = escape_attr(&rel.thumbnail_url);

        let badge = if !r_duration.is_empty() {
            format!(
                r#"<span style="position: absolute; bottom: 4px; right: 4px; background: rgba(0,0,0,0.8); color: #fff; padding: 1px 3px; border-radius: 4px; font-size: 11px;">{r_duration}</span>"#
            )
        } else {
            String::new()
        };

        related_html.push_str(&format!(
            r#"<div style="display: flex; gap: 8px; margin-bottom: 12px;">
  <a href="/watch?v={r_vid}" style="flex-shrink: 0; width: 168px; height: 94px; border-radius: 8px; overflow: hidden; position: relative; display: block;">
    <img src="{r_thumb}" style="width: 100%; height: 100%; object-fit: cover; display: block;" alt="{r_title}">
    {badge}
  </a>
  <div style="display: flex; flex-direction: column; overflow: hidden;">
    <a href="/watch?v={r_vid}" title="{r_title}" style="color: var(--yt-spec-text-primary, #f1f1f1); font-size: 14px; line-height: 18px; font-weight: 500; text-decoration: none; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;">
      {r_title}
    </a>
    <span style="color: var(--yt-spec-text-secondary, #aaa); font-size: 12px; margin-top: 4px;">{r_channel}</span>
    <span style="color: var(--yt-spec-text-secondary, #aaa); font-size: 12px;">{r_views}</span>
  </div>
</div>"#
        ));
    }

    format!(
        r#"<div id="page-manager" class="style-scope ytd-app" style="margin-top: 56px; padding: 24px 32px; display: flex; flex-direction: row; gap: 24px; max-width: 1800px; margin-left: auto; margin-right: auto;">
  <div id="primary" style="flex: 1; min-width: 0;">
    <div id="player-container" style="width: 100%; aspect-ratio: 16/9; background: #000; border-radius: 12px; overflow: hidden; position: relative; display: flex; align-items: center; justify-content: center;">
      <img src="https://i.ytimg.com/vi/{vid}/hqdefault.jpg" style="width: 100%; height: 100%; object-fit: cover;" alt="{title}">
      <div style="position: absolute; width: 68px; height: 48px; background: rgba(33,33,33,0.8); border-radius: 12px; display: flex; align-items: center; justify-content: center;">
        <svg viewBox="0 0 24 24" style="width: 28px; height: 28px; fill: #fff;"><path d="M8 5v14l11-7z"/></svg>
      </div>
    </div>
    <div id="video-info" style="margin-top: 16px;">
      <h1 style="font-size: 20px; line-height: 28px; font-weight: 600; margin: 0 0 10px 0; color: var(--yt-spec-text-primary, #f1f1f1);">{title}</h1>
      <div style="display: flex; align-items: center; justify-content: space-between; border-bottom: 1px solid rgba(255,255,255,0.1); padding-bottom: 12px;">
        <div style="display: flex; align-items: center; gap: 12px;">
          <div style="width: 40px; height: 40px; border-radius: 50%; background: #444;"></div>
          <div>
            <div style="font-weight: 500; font-size: 15px; color: var(--yt-spec-text-primary, #f1f1f1);">{channel}</div>
            <div style="font-size: 12px; color: var(--yt-spec-text-secondary, #aaa);">{subs}</div>
          </div>
        </div>
        <div style="color: var(--yt-spec-text-secondary, #aaa); font-size: 14px;">{views}</div>
      </div>
    </div>
  </div>
  <div id="secondary" style="width: 400px; flex-shrink: 0;">
    <h3 style="font-size: 16px; margin: 0 0 16px 0; color: var(--yt-spec-text-primary, #f1f1f1);">Related Videos</h3>
    {related_html}
  </div>
</div>"#
    )
}

pub fn maybe_hydrate_youtube_html(url: &str, html: &str) -> Option<String> {
    if !url.contains("youtube.com") && !url.contains("youtu.be") {
        return None;
    }

    let marker = "var ytInitialData = ";
    let pos = html.find(marker)?;
    let start_json = pos + marker.len();
    let end_script = html[start_json..].find("</script>")? + start_json;

    let json_slice = html[start_json..end_script].trim();
    let json_clean = json_slice.strip_suffix(';').unwrap_or(json_slice).trim();

    let data: Value = serde_json::from_str(json_clean).ok()?;
    let contents = data.get("contents")?;

    let hydrated_page_manager = if let Some(two_col_search) = contents.get("twoColumnSearchResultsRenderer") {
        let sec_contents = two_col_search
            .get("primaryContents")
            .and_then(|pc| pc.get("sectionListRenderer"))
            .and_then(|sl| sl.get("contents"))
            .and_then(|c| c.as_array())?;

        let mut items = Vec::new();
        for sec in sec_contents {
            if let Some(item_sec) = sec.get("itemSectionRenderer").and_then(|is| is.get("contents")).and_then(|c| c.as_array()) {
                for item in item_sec {
                    if let Some(vr) = item.get("videoRenderer") {
                        if let Some(parsed) = parse_video_renderer(vr) {
                            items.push(parsed);
                        }
                    }
                }
            }
        }

        let mut cards_html = String::new();
        for item in &items {
            cards_html.push_str(&render_search_card(item));
        }

        format!(
            r#"<div id="page-manager" class="style-scope ytd-app" style="margin-top: 56px; padding: 16px 24px; max-width: 1280px; margin-left: auto; margin-right: auto;">
  <ytd-search class="style-scope ytd-page-manager">
    <div id="primary" class="style-scope ytd-search">
      <ytd-section-list-renderer class="style-scope ytd-search">
        <div id="contents" class="style-scope ytd-section-list-renderer">
          <ytd-item-section-renderer class="style-scope ytd-section-list-renderer">
            <div id="contents" class="style-scope ytd-item-section-renderer">
              {cards_html}
            </div>
          </ytd-item-section-renderer>
        </div>
      </ytd-section-list-renderer>
    </div>
  </ytd-search>
</div>"#
        )
    } else if let Some(watch_results) = contents.get("twoColumnWatchNextResults") {
        let results_list = watch_results
            .get("results")
            .and_then(|r| r.get("results"))
            .and_then(|r| r.get("contents"))
            .and_then(|c| c.as_array());

        let mut title = String::new();
        let mut channel = String::new();
        let mut views = String::new();
        let mut subs = String::new();

        if let Some(list) = results_list {
            for item in list {
                if let Some(vpr) = item.get("videoPrimaryInfoRenderer") {
                    title = vpr.get("title").map(extract_text_from_runs).unwrap_or_default();
                    views = vpr.get("viewCount").and_then(|vc| vc.get("videoViewCountRenderer")).map(extract_text_from_runs).unwrap_or_default();
                }
                if let Some(vsr) = item.get("videoSecondaryInfoRenderer") {
                    if let Some(owner) = vsr.get("owner").and_then(|o| o.get("videoOwnerRenderer")) {
                        channel = owner.get("title").map(extract_text_from_runs).unwrap_or_default();
                        subs = owner.get("subscriberCountText").map(extract_text_from_runs).unwrap_or_default();
                    }
                }
            }
        }

        let mut related_videos = Vec::new();
        if let Some(secondary) = watch_results
            .get("secondaryResults")
            .and_then(|sr| sr.get("secondaryResults"))
            .and_then(|sr| sr.get("results"))
            .and_then(|r| r.as_array())
        {
            for item in secondary {
                if let Some(cvr) = item.get("compactVideoRenderer") {
                    if let Some(parsed) = parse_video_renderer(cvr) {
                        related_videos.push(parsed);
                    }
                }
            }
        }

        let video_id = url
            .split("v=")
            .nth(1)
            .and_then(|s| s.split('&').next())
            .unwrap_or("")
            .to_string();

        render_watch_page(&WatchPageInfo {
            video_id,
            title,
            channel,
            views,
            subscribers: subs,
            related_videos,
        })
    } else if let Some(two_col_browse) = contents.get("twoColumnBrowseResultsRenderer") {
        let tabs = two_col_browse.get("tabs").and_then(|t| t.as_array())?;
        let tab0 = tabs.first()?.get("tabRenderer")?;
        let content = tab0.get("content")?;
        let grid = content.get("richGridRenderer")?;
        let grid_contents = grid.get("contents").and_then(|c| c.as_array())?;

        let mut video_items = Vec::new();
        let mut nudge_info = None;

        for item in grid_contents {
            if let Some(rich_item) = item.get("richItemRenderer") {
                if let Some(vr) = rich_item.get("content").and_then(|c| c.get("videoRenderer")) {
                    if let Some(parsed) = parse_video_renderer(vr) {
                        video_items.push(parsed);
                    }
                }
            } else if let Some(rich_section) = item.get("richSectionRenderer") {
                if let Some(feed_nudge) = rich_section.get("content").and_then(|c| c.get("feedNudgeRenderer")) {
                    let n_title = feed_nudge.get("title").map(extract_text_from_runs).unwrap_or_default();
                    let n_sub = feed_nudge.get("subtitle").map(extract_text_from_runs).unwrap_or_default();
                    nudge_info = Some(NudgeInfo {
                        title: n_title,
                        subtitle: n_sub,
                    });
                }
            }
        }

        let inner_html = if !video_items.is_empty() {
            let mut cards_html = String::new();
            for item in &video_items {
                cards_html.push_str(&render_grid_card(item));
            }
            format!(
                r#"<div id="contents" class="style-scope ytd-rich-grid-renderer" style="display: flex; flex-wrap: wrap; gap: 16px; padding: 24px; max-width: 1800px; margin: 0 auto; justify-content: flex-start;">
  {cards_html}
</div>"#
            )
        } else if let Some(nudge) = &nudge_info {
            render_feed_nudge(nudge)
        } else {
            String::new()
        };

        format!(
            r#"<div id="page-manager" class="style-scope ytd-app" style="margin-top: 56px;">
  <ytd-browse class="style-scope ytd-page-manager">
    <ytd-two-column-browse-results-renderer class="style-scope ytd-browse">
      <div id="primary" class="style-scope ytd-two-column-browse-results-renderer">
        <ytd-rich-grid-renderer class="style-scope ytd-two-column-browse-results-renderer">
          {inner_html}
        </ytd-rich-grid-renderer>
      </div>
    </ytd-two-column-browse-results-renderer>
  </ytd-browse>
</div>"#
        )
    } else {
        return None;
    };

    let hydration_style = r#"<style id="rix-youtube-fastpath">
  #home-page-skeleton, #watch-page-skeleton, #home-container-skeleton, #guide-skeleton { display: none !important; }
  ytd-app #page-manager { display: block !important; }
  .ytCoreImageHost, img.style-scope.yt-img-shadow, ytd-thumbnail img { visibility: visible !important; opacity: 1 !important; }
  ytd-masthead { position: fixed !important; top: 0 !important; left: 0 !important; right: 0 !important; z-index: 2000 !important; }
</style>"#;

    let mut result = html.to_string();

    // Mark base-js script as deferred to avoid blocking HTML parser stream during initial render
    if let Some(base_js_pos) = result.find("id=\"base-js\"") {
        if !result[..base_js_pos].ends_with("defer ") && !result[base_js_pos..].starts_with("id=\"base-js\" defer") {
            result.insert_str(base_js_pos + 12, " defer");
        }
    }

    if let Some(ytd_app_end) = result.find("</ytd-app>") {
        result.insert_str(ytd_app_end, &format!("{hydration_style}\n{hydrated_page_manager}\n"));
    } else if let Some(body_end) = result.find("</body>") {
        result.insert_str(body_end, &format!("{hydration_style}\n{hydrated_page_manager}\n"));
    } else {
        result.push_str(hydration_style);
        result.push_str(&hydrated_page_manager);
    }

    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_youtube_url_is_ignored() {
        let html = "<html><body><h1>Hello</h1></body></html>";
        assert!(maybe_hydrate_youtube_html("https://example.com/", html).is_none());
    }

    #[test]
    fn test_hydrate_search_results() {
        let fake_json = r#"{"contents":{"twoColumnSearchResultsRenderer":{"primaryContents":{"sectionListRenderer":{"contents":[{"itemSectionRenderer":{"contents":[{"videoRenderer":{"videoId":"test1234","title":{"runs":[{"text":"Rust Fast-Path"}]},"ownerText":{"runs":[{"text":"Ferris Channel"}]},"viewCountText":{"simpleText":"100K views"},"lengthText":{"simpleText":"10:00"},"thumbnail":{"thumbnails":[{"url":"https://i.ytimg.com/vi/test1234/hqdefault.jpg"}]}}}]}}]}}}}}"#;
        let html = format!(
            "<html><body><ytd-app><ytd-masthead id=\"masthead\"></ytd-masthead></ytd-app><script>var ytInitialData = {fake_json};</script></body></html>"
        );
        let hydrated = maybe_hydrate_youtube_html("https://www.youtube.com/results?search_query=rust", &html).expect("must hydrate");
        assert!(hydrated.contains("Rust Fast-Path"));
        assert!(hydrated.contains("Ferris Channel"));
        assert!(hydrated.contains("10:00"));
        assert!(hydrated.contains("test1234"));
        assert!(hydrated.contains("rix-youtube-fastpath"));
    }

    #[test]
    fn test_hydrate_browse_grid() {
        let fake_json = r#"{"contents": {"twoColumnBrowseResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"richGridRenderer": {"contents": [{"richItemRenderer": {"content": {"videoRenderer": {"videoId": "home999", "title": {"runs": [{"text": "Home Trending Video"}]}, "ownerText": {"runs": [{"text": "Tech Studio"}]}, "viewCountText": {"simpleText": "500K views"}, "lengthText": {"simpleText": "15:30"}, "thumbnail": {"thumbnails": [{"url": "https://i.ytimg.com/vi/home999/hqdefault.jpg"}]}}}}}]}}}}]}}}"#;
        let html = format!(
            "<html><body><ytd-app></ytd-app><script>var ytInitialData = {fake_json};</script></body></html>"
        );
        let hydrated = maybe_hydrate_youtube_html("https://www.youtube.com/", &html).expect("must hydrate");
        assert!(hydrated.contains("Home Trending Video"));
        assert!(hydrated.contains("Tech Studio"));
        assert!(hydrated.contains("home999"));
    }

    #[test]
    fn test_hydrate_feed_nudge() {
        let fake_json = r#"{"contents":{"twoColumnBrowseResultsRenderer":{"tabs":[{"tabRenderer":{"content":{"richGridRenderer":{"contents":[{"richSectionRenderer":{"content":{"feedNudgeRenderer":{"title":{"runs":[{"text":"Try searching to get started"}]},"subtitle":{"runs":[{"text":"Start watching videos"}]}}}}}]}}}}]}}}"#;
        let html = format!(
            "<html><body><ytd-app></ytd-app><script>var ytInitialData = {fake_json};</script></body></html>"
        );
        let hydrated = maybe_hydrate_youtube_html("https://www.youtube.com/", &html).expect("must hydrate");
        assert!(hydrated.contains("Try searching to get started"));
        assert!(hydrated.contains("Start watching videos"));
    }
}
