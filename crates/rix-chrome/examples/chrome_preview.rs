use rix_chrome::customization::{
    CustomizationConfig, CustomizationManager, TabOrientation, ThemeMode,
};
use rix_chrome::shell::BrowserShell;
use rix_render::VelloRenderer;
use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("rix-chrome-preview"));
    std::fs::create_dir_all(&output)?;
    let temp = tempfile::tempdir()?;
    let css_path = temp.path().join("userCustom.css");
    std::fs::write(&css_path, "")?;
    let cases = [
        (
            "dark-horizontal",
            1280,
            ThemeMode::Dark,
            TabOrientation::Horizontal,
        ),
        (
            "light-horizontal",
            1280,
            ThemeMode::Light,
            TabOrientation::Horizontal,
        ),
        (
            "dark-vertical",
            1280,
            ThemeMode::Dark,
            TabOrientation::Vertical,
        ),
        (
            "light-vertical",
            1280,
            ThemeMode::Light,
            TabOrientation::Vertical,
        ),
        (
            "narrow-many-tabs",
            480,
            ThemeMode::Dark,
            TabOrientation::Horizontal,
        ),
    ];
    for (name, width, theme, orientation) in cases {
        let customization = CustomizationManager::new(CustomizationConfig {
            theme_mode: theme,
            tab_orientation: orientation,
            custom_css_path: css_path.clone(),
        });
        let mut shell = BrowserShell::new_with_customization(width as f32, 800.0, customization)?;
        shell.navigate_active_tab("https://www.google.com").await;
        let count = if width < 600 { 6 } else { 4 };
        for _ in 1..count {
            shell.create_new_tab("about:blank").await;
        }
        {
            let mut host = shell.priv_state.borrow_mut();
            for (index, tab) in host.tabs.iter_mut().enumerate() {
                tab.title = match index {
                    0 => "RixBrowser — Home".to_string(),
                    1 => "Documentation — a long page title used to check tab truncation"
                        .to_string(),
                    2 => "A long article title that should fit neatly in a compact browser tab"
                        .to_string(),
                    _ => format!("New tab {}", index + 1),
                };
            }
        }
        shell.switch_tab(1);
        shell.recompute_layout()?;
        let (scene, _) = shell.render_frame();
        let path = output.join(format!("{name}.png"));
        VelloRenderer::render_scene_to_png(&scene, width, 800, &path).await?;
        println!("{}", path.display());
    }
    Ok(())
}
