fn main() {
    let mut config = prost_build::Config::new();

    config.type_attribute(
        "clipx.DeviceType",
        "#[derive(serde::Serialize, serde::Deserialize)]",
    );
    
    config
        .compile_protos(&["../../protos/clipx.proto"], &["../../protos/"])
        .unwrap();
    println!("cargo:rerun-if-changed=../../protos/clipx.proto");
}
