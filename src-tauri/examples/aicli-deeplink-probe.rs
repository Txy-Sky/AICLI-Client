//! CI-only probe: register the real plugin in an isolated HOME, without windows.
use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(std::env::var("HOME")?, std::env::var("CC_SWITCH_TEST_HOME")?);
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_deep_link::init())
        .build(context)?;
    app.deep_link().register_all()?;
    let handler = app
        .path()
        .data_dir()?
        .join("applications/aicli-client-handler.desktop");
    let content = std::fs::read_to_string(&handler)?;
    assert!(content.contains("x-scheme-handler/aicliapi"));
    assert!(!content.contains("x-scheme-handler/ccswitch"));
    println!("B-04 generated handler: {}", handler.display());
    println!("B-04 handler scheme: aicliapi");
    Ok(())
}
