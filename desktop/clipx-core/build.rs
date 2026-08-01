fn main() {
    prost_build::compile_protos(&["../../protos/clipboard.proto"], &["../../protos/"])
        .unwrap();
}