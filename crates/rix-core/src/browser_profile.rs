
use std::env;
use std::thread;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserProfile {
    user_agent: String,
    languages: Vec<String>,
    platform: String,
    hardware_concurrency: usize,
}

impl BrowserProfile {
    pub fn system_default() -> Self {
        let platform = system_platform();
        let user_agent = format!(
            "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36 RixBrowser/{}",
            user_agent_platform(&platform),
            env!("CARGO_PKG_VERSION")
        );
        let hardware_concurrency = thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1);

        Self::new(
            user_agent,
            system_languages(),
            platform,
            hardware_concurrency,
        )
    }

    pub fn new(
        user_agent: impl Into<String>,
        languages: impl IntoIterator<Item = String>,
        platform: impl Into<String>,
        hardware_concurrency: usize,
    ) -> Self {
        let languages = normalize_language_preferences(languages);
        Self {
            user_agent: user_agent.into(),
            languages,
            platform: platform.into(),
            hardware_concurrency: hardware_concurrency.max(1),
        }
    }

    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    pub fn language(&self) -> &str {
        self.languages
            .first()
            .map(String::as_str)
            .unwrap_or("en-US")
    }

    pub fn languages(&self) -> &[String] {
        &self.languages
    }

    pub fn accept_language(&self) -> String {
        self.languages
            .iter()
            .take(10)
            .enumerate()
            .map(|(index, language)| {
                if index == 0 {
                    language.clone()
                } else {
                    let quality = 1.0 - index as f64 / 10.0;
                    format!("{language};q={quality:.1}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn platform(&self) -> &str {
        &self.platform
    }

    pub fn hardware_concurrency(&self) -> usize {
        self.hardware_concurrency
    }

    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }
}

fn system_languages() -> Vec<String> {
    let mut preferences = Vec::new();
    if let Some(value) = env::var_os("LANGUAGE").and_then(|value| value.into_string().ok()) {
        preferences.extend(value.split(':').map(str::to_owned));
    }
    for name in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(value) = env::var_os(name).and_then(|value| value.into_string().ok()) {
            preferences.push(value);
        }
    }
    let normalized = normalize_language_preferences(preferences);
    if normalized.is_empty() {
        vec!["en-US".to_owned()]
    } else {
        normalized
    }
}

fn normalize_language_preferences(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut languages = Vec::new();
    for value in values {
        for part in value.split(':') {
            let Some(tag) = normalize_language_tag(part) else {
                continue;
            };
            if !languages.iter().any(|existing| existing == &tag) {
                languages.push(tag);
            }
        }
    }
    if languages.is_empty() {
        languages.push("en-US".to_owned());
    }
    languages
}

fn normalize_language_tag(locale: &str) -> Option<String> {
    let locale = locale.trim().split(['.', '@']).next()?.trim();
    if locale.eq_ignore_ascii_case("c") || locale.eq_ignore_ascii_case("posix") {
        return None;
    }

    let mut parts = locale.split(['-', '_']);
    let language = parts.next()?;
    if !(2..=8).contains(&language.len())
        || !language.bytes().all(|byte| byte.is_ascii_alphabetic())
    {
        return None;
    }
    let mut normalized = vec![language.to_ascii_lowercase()];
    for part in parts {
        if part.is_empty()
            || part.len() > 8
            || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return None;
        }
        let formatted = if part.len() == 4 && part.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            let mut chars = part.chars();
            chars.next().map(|first| {
                first.to_ascii_uppercase().to_string() + &chars.as_str().to_ascii_lowercase()
            })?
        } else if (part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
            || (part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            part.to_ascii_uppercase()
        } else {
            part.to_ascii_lowercase()
        };
        normalized.push(formatted);
    }
    Some(normalized.join("-"))
}

fn system_platform() -> String {
    match env::consts::OS {
        "linux" => format!("Linux {}", env::consts::ARCH),
        "windows" => format!("Win32 ({})", env::consts::ARCH),
        "macos" => format!("Macintosh ({})", env::consts::ARCH),
        os => format!("{os} ({})", env::consts::ARCH),
    }
}

fn user_agent_platform(platform: &str) -> String {
    if platform.starts_with("Linux ") {
        format!("X11; {platform}")
    } else {
        platform.to_owned()
    }
}
