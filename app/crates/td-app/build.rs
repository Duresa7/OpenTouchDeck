fn main() {
    // Debug builds keep element ids and the tree data the headless UI tests search; release
    // builds leave them out.
    let debug = std::env::var("PROFILE").is_ok_and(|profile| profile == "debug");
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_debug_info(debug);
    slint_build::compile_with_config("ui/app.slint", config).expect("the Slint UI compiles");
    // The About licence test resolves this target's dependencies.
    let target = std::env::var("TARGET").expect("cargo sets TARGET");
    println!("cargo:rustc-env=TD_APP_TARGET={target}");
}
