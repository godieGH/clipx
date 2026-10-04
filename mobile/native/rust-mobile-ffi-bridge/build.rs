fn main() {
    println!("cargo:rerun-if-env-changed=TARGET");

    let target = std::env::var("TARGET").unwrap_or_default();

    // android build with 16KB page alignment
    if target.contains("android") {
        println!("cargo:rustc-link-arg=-Wl,-z,max-page-size=16384");
    }
}
