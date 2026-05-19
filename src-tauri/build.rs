fn main() {
    tauri_build::build();
    tonic_build::configure()
        .build_server(false)
        .compile_protos(
            &["../proto/spacex/api/device/device.proto"],
            &["../proto"],
        )
        .expect("Failed to compile protos");
}
