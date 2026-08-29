
fn main() {
    println!("cargo:rerun-if-env-changed=TARGET");

    let target = std::env::var("TARGET").unwrap_or_default();

    if target.contains("android") {
        println!("cargo:rustc-link-arg=-Wl,-z,max-page-size=16384");
        
        // Option B: If building directly via rust-lld without clang:
        // println!("cargo:rustc-link-arg=-z");
        // println!("cargo:rustc-link-arg=max-page-size=16384");
    }
}
