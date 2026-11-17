fn main() {
    println!("cargo:rustc-check-cfg=cfg(codisseu_tab_lcd)");
    println!("cargo:rerun-if-changed=src/tab_lcd.c");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux")
        || std::env::var_os("CARGO_FEATURE_TAB_LCD").is_none()
    {
        return;
    }
    // Optional host adapter: absence preserves the portable egui renderer.
    match pkg_config::Config::new().probe("freetype2") {
        Ok(lib) => {
            let mut build = cc::Build::new();
            build.file("src/tab_lcd.c").includes(lib.include_paths);
            build.warnings_into_errors(true).compile("codisseu_tab_lcd");
            println!("cargo:rustc-cfg=codisseu_tab_lcd");
        }
        Err(_) => println!("cargo:warning=FreeType unavailable; using portable egui tab labels"),
    }
}
