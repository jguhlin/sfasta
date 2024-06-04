use std::env;
use std::path::PathBuf;

#[cfg(feature = "bindgen")]
fn gen_bindings() {
    let out_path = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    let bindgen = bindgen::Builder::default()
    .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));

    let bindgen = bindgen.header("htscodecs.h");
   
    bindgen
        .generate_cstr(true)
        .generate()
        .expect("Couldn't write bindings!")
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Unable to create bindings");
}

fn compile() {
    let src = [
        "htscodecs/htscodecs/fqzcomp_qual.c",
        // "htscodecs/htscodecs/utils.c",

    ];
    let mut builder = cc::Build::new();
    let build = builder
        .files(src.iter())
        .include("include")
        .include("htscodecs/htscodecs")
        .flag("-Wno-unused-parameter")
        .define("USE_ZLIB", None);

    build.compile("libhtscodecs");
}

#[cfg(not(feature = "bindgen"))]
fn gen_bindings() {}

fn main() {
    compile();
    gen_bindings();
}