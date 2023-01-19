use std::io;
#[cfg(windows)] use winres::WindowsResource;

fn main() -> io::Result<()> {
    tonic_build::compile_protos("proto/helloworld.proto")?;

    #[cfg(windows)] {
        WindowsResource::new()
            // This path can be absolute, or relative to your crate root.
            .set_icon_with_id("icon.ico", "2")
            .compile()?;
    }
    Ok(())
}
