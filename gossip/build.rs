use std::io::Result;
fn main() -> Result<()> {
    prost_build::compile_protos(&["src/communication/p2p/message.proto"], &["src/"])?;
    Ok(())
}
