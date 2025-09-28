use std::path::PathBuf;
use std::{env, io};

#[cfg(windows)]
use winres::WindowsResource;

fn main() -> io::Result<()> {
    let descriptor_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("descriptor.bin");

    let protos = glob::glob("./proto/rumgap/**/*.proto")
        .unwrap()
        .collect::<Result<Vec<PathBuf>, glob::GlobError>>()
        .unwrap();

    tonic_prost_build::configure()
        .file_descriptor_set_path(descriptor_path)
        .build_client(false)
        .compile_well_known_types(false)
        .protoc_arg("--experimental_allow_proto3_optional")
        .compile_protos(protos.as_slice(), &["proto".into(), "/usr/local/include".into()])?;

    #[cfg(windows)]
    {
        WindowsResource::new()
            // This path can be absolute, or relative to your crate root.
            .set_icon_with_id("icon.ico", "2")
            .compile()?;
    }
    Ok(())
}
