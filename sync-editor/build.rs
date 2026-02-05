//! Build script for sync-editor
//!
//! This configures Tauri when the `tauri` feature is enabled.

fn main() {
    #[cfg(feature = "tauri")]
    tauri_build::build();
}
