fn main() {
    let mut config = prost_build::Config::new();

    config.type_attribute(
        "clipx.DeviceType",
        "#[derive(serde::Serialize, serde::Deserialize)]",
    );

    // Needed so PairingEvent can be passed directly as a Tauri event
    // payload (app_handle.emit) instead of being flattened to `()`.
    config.type_attribute("clipx.PairingEvent", "#[derive(serde::Serialize)]");
    config.type_attribute("clipx.PairingEvent.State", "#[derive(serde::Serialize)]");
    config.type_attribute("clipx.FileTransferEvent", "#[derive(serde::Serialize)]");

    config
        .compile_protos(&["../../protos/clipx.proto"], &["../../protos/"])
        .unwrap();
    println!("cargo:rerun-if-changed=../../protos/clipx.proto");
}
