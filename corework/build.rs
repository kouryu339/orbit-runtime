fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);

    tonic_prost_build::configure()
        .build_server(false)
        .build_client(std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("wasm32"))
        .compile_protos(&["proto/corework_agent_tool_v1.proto"], &["proto"])?;

    println!("cargo:rerun-if-changed=proto/corework_agent_tool_v1.proto");
    Ok(())
}
