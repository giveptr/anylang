use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use std::io::{Read, Write};

pub fn opened(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    ZlibDecoder::new(body)
        .read_to_end(&mut out)
        .map_err(|why| format!("it would not unpack: {why}"))?;

    Ok(out)
}

pub fn shut(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut packer = ZlibEncoder::new(Vec::new(), Compression::default());

    packer
        .write_all(body)
        .and_then(|()| packer.finish())
        .map_err(|why| format!("it would not pack: {why}"))
}
