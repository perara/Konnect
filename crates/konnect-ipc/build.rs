fn main() -> Result<(), Box<dyn std::error::Error>> {
    // nng-sys's Windows IPC listener references IsValidSecurityDescriptor
    // (advapi32) but doesn't always emit the link directive itself — without
    // this, test executables that pull in the listener object can fail with
    // LNK2019 unresolved externals.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=advapi32");
    }

    let protos = &[
        "proto/common/envelope.proto",
        "proto/common/types/base_types.proto",
        "proto/common/types/enums.proto",
        "proto/common/types/project_settings.proto",
        "proto/common/commands/base_commands.proto",
        "proto/common/commands/editor_commands.proto",
        "proto/common/commands/project_commands.proto",
        "proto/board/board.proto",
        "proto/board/board_commands.proto",
        "proto/board/board_types.proto",
    ];

    // Include paths: our proto dir + protoc's well-known types. setup-protoc
    // places the includes next to its binary, while distro packages normally use
    // /usr/include or /usr/local/include. PROTOC_INCLUDE lets unusual layouts opt in.
    let protoc_path = std::env::var("PROTOC").unwrap_or_else(|_| "protoc".to_string());
    let protoc_dir = std::path::Path::new(&protoc_path)
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("include"))
        .unwrap_or_default();

    let mut includes = vec![std::path::PathBuf::from("proto/")];
    if protoc_dir.join("google/protobuf/any.proto").is_file() {
        includes.push(protoc_dir);
    }
    if let Some(include) = std::env::var_os("PROTOC_INCLUDE") {
        let include = std::path::PathBuf::from(include);
        if include.join("google/protobuf/any.proto").is_file() {
            includes.push(include);
        }
    }
    for include in ["/usr/local/include", "/usr/include"] {
        let include = std::path::PathBuf::from(include);
        if include.join("google/protobuf/any.proto").is_file() {
            includes.push(include);
        }
    }

    prost_build::Config::new().compile_protos(protos, &includes)?;

    Ok(())
}
