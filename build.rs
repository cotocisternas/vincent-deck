fn main() {
    println!("cargo:rerun-if-changed=src/audio/native.c");
    let library = pkg_config::Config::new()
        .atleast_version("0.5")
        .probe("wireplumber-0.5")
        .expect("WirePlumber 0.5 development headers are required");
    cc::Build::new()
        .file("src/audio/native.c")
        .includes(library.include_paths)
        .warnings(true)
        .flag("-Wall")
        .flag("-Wextra")
        .compile("vincent_audio");
}
