fn main() {
    // A custom manifest makes Windows ask for elevation before the process starts,
    // so the bridge always runs with administrator rights (requireAdministrator).
    let attributes = tauri_build::Attributes::new().windows_attributes(
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("windows-app-manifest.xml")),
    );
    tauri_build::try_build(attributes).expect("tauri-build failed");
}
