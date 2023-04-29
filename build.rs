use std::path::PathBuf;
use std::{env, io};

#[cfg(windows)]
use winres::WindowsResource;

fn main() -> io::Result<()> {
    let descriptor_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("descriptor.bin");

    tonic_build::configure()
        .file_descriptor_set_path(descriptor_path)
        .compile(&["proto/rumgap/v1/rumgap.proto"], &["proto"])?;

    #[cfg(windows)]
    {
        WindowsResource::new()
            // This path can be absolute, or relative to your crate root.
            .set_icon_with_id("icon.ico", "2")
            .compile()?;
    }
    Ok(())
}
