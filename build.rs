use std::path::{PathBuf, Path};
use std::{env, io};

#[cfg(windows)]
use winres::WindowsResource;

fn main() -> io::Result<()> {
    let descriptor_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("descriptor.bin");

    let protos = glob::glob("./proto/rumgap/**/*.proto")
        .unwrap()
        .collect::<Result<Vec<PathBuf>, glob::GlobError>>()
        .unwrap();

    tonic_build::configure()
        .file_descriptor_set_path(descriptor_path)
        .build_client(false)
        .compile(protos.as_slice(), &["proto"])?;

    #[cfg(windows)]
    {
        WindowsResource::new()
            // This path can be absolute, or relative to your crate root.
            .set_icon_with_id("icon.ico", "2")
            .compile()?;
    }
    Ok(())
}
