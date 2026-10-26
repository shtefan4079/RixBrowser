
#![deny(unsafe_code)]

use crate::config::{AppConfig, AppTabOrientation, AppTheme, DEFAULT_CONTROL_PORT};

#[derive(Debug, Clone, PartialEq)]
pub struct CliArgs {
    pub show_version: bool,
    pub show_help: bool,
    pub headless: bool,
    pub windowed: bool,
    pub control_port: Option<u16>,
    pub initial_url: Option<String>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub theme: Option<AppTheme>,
    pub tab_orientation: Option<AppTabOrientation>,
}

impl CliArgs {
    pub fn parse_from<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let collected: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();
        Self::parse_slice(&collected)
    }

    fn parse_slice(args: &[String]) -> Result<Self, String> {
        let mut out = Self {
            show_version: false,
            show_help: false,
            headless: false,
            windowed: false,
            control_port: None,
            initial_url: None,
            width: None,
            height: None,
            theme: None,
            tab_orientation: None,
        };

        let mut i = 1usize;
        while i < args.len() {
            match args[i].as_str() {
                "--version" | "-v" => out.show_version = true,
                "--help" | "-h" => out.show_help = true,
                "--headless" => out.headless = true,
                "--windowed" | "--no-headless" => out.windowed = true,
                "--control-port" | "--remote-debugging-port" => {
                    let v = args
                        .get(i + 1)
                        .ok_or_else(|| format!("{} requires a port number", args[i]))?;
                    out.control_port = Some(
                        v.parse::<u16>()
                            .map_err(|_| format!("invalid port number: {v}"))?,
                    );
                    i += 1;
                }
                "--url" => {
                    let v = args
                        .get(i + 1)
                        .ok_or_else(|| "--url requires a value".to_string())?;
                    out.initial_url = Some(v.clone());
                    i += 1;
                }
                "--width" => {
                    let v = args
                        .get(i + 1)
                        .ok_or_else(|| "--width requires a value".to_string())?;
                    out.width = Some(
                        v.parse::<f32>()
                            .map_err(|_| format!("invalid --width value: {v}"))?,
                    );
                    i += 1;
                }
                "--height" => {
                    let v = args
                        .get(i + 1)
                        .ok_or_else(|| "--height requires a value".to_string())?;
                    out.height = Some(
                        v.parse::<f32>()
                            .map_err(|_| format!("invalid --height value: {v}"))?,
                    );
                    i += 1;
                }
                "--theme" => {
                    let v = args
                        .get(i + 1)
                        .ok_or_else(|| "--theme requires dark|light|custom".to_string())?;
                    out.theme = Some(match v.to_lowercase().as_str() {
                        "dark" => AppTheme::Dark,
                        "light" => AppTheme::Light,
                        "custom" => AppTheme::Custom,
                        _ => {
                            return Err(format!(
                                "invalid --theme value: {v} (expected dark|light|custom)"
                            ))
                        }
                    });
                    i += 1;
                }
                "--vertical-tabs" => out.tab_orientation = Some(AppTabOrientation::Vertical),
                "--horizontal-tabs" => out.tab_orientation = Some(AppTabOrientation::Horizontal),
                unknown if unknown.starts_with('-') => {
                    return Err(format!("unknown flag: {unknown}\n{}", Self::usage()));
                }
                _ => {
                    if out.initial_url.is_none() {
                        out.initial_url = Some(args[i].clone());
                    }
                }
            }
            i += 1;
        }

        Ok(out)
    }

    pub fn apply_to_config(&self, mut base: AppConfig) -> AppConfig {
        if self.headless {
            base.headless = true;
        }
        if self.windowed {
            base.headless = false;
        }
        if let Some(port) = self.control_port {
            base.control_port = port;
        }
        if let Some(ref url) = self.initial_url {
            base.homepage = normalize_initial_url(url);
        }
        if let Some(w) = self.width {
            base.window_width = w;
        }
        if let Some(h) = self.height {
            base.window_height = h;
        }
        if let Some(theme) = self.theme {
            base.theme = theme;
        }
        if let Some(orientation) = self.tab_orientation {
            base.tab_orientation = orientation;
        }
        base.validate();
        base
    }

    pub fn effective_control_port(&self, fallback: u16) -> u16 {
        self.control_port.unwrap_or(fallback)
    }

    pub fn usage() -> &'static str {
        "Usage: rix-browser [OPTIONS] [URL]\n\
         \n\
         Options:\n  \
         --headless                    Run without a native window (automation/CI)\n  \
         --windowed, --no-headless      Force native window even if config has headless=true\n  \
         --version, -v                 Print version and exit\n  \
         --control-port <PORT>         Automation HTTP API port (default 9222)\n  \
         --remote-debugging-port <P>   Alias for --control-port\n  \
         --url <URL>                   Initial address or search query to open\n  \
         --width <PX> --height <PX>    Initial window geometry\n  \
         --theme <dark|light|custom>   Chrome shell theme\n  \
         --vertical-tabs               Vertical tab rail on the left\n  \
         --horizontal-tabs             Horizontal tab strip on top (default)\n  \
         --help, -h                    Print this help and exit"
    }
}

fn normalize_initial_url(raw: &str) -> String {
    let s = raw.trim().to_string();
    if s.contains("://")
        || s.starts_with("about:")
        || s.starts_with("data:")
        || s.starts_with("rix://")
    {
        return s;
    }
    if s.chars().any(char::is_whitespace) {
        return s;
    }
    format!("https://{s}")
}

pub fn resolve_control_port(args: &[String], fallback: u16) -> u16 {
    let mut port = fallback;
    let mut iter = args.windows(2);
    let mut i = 0usize;
    while i + 1 < args.len() {
        if args[i] == "--control-port" || args[i] == "--remote-debugging-port" {
            if let Ok(p) = args[i + 1].parse::<u16>() {
                port = p;
            }
        }
        i += 1;
    }
    let _ = iter.next();
    if port == 0 {
        return DEFAULT_CONTROL_PORT;
    }
    port
}
