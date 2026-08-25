use crate::canvas::{Canvas, smaller};
use jpeg_decoder::{Decoder, PixelFormat};
use std::io::Cursor;

pub fn drawn(raw: &[u8], most: Option<usize>) -> Option<Canvas> {
    let mut held = Decoder::new(Cursor::new(raw));
    held.read_info().ok()?;

    let told = held.info()?;
    let asked =
        most.and_then(|most| smaller(usize::from(told.width), usize::from(told.height), most));
    let (wide, high) = match asked {
        Some((wide, high)) => held
            .scale(u16::try_from(wide).ok()?, u16::try_from(high).ok()?)
            .ok()?,
        None => (told.width, told.height),
    };

    let pixels = held.decode().ok()?;
    let (wide, high) = (usize::from(wide), usize::from(high));

    Canvas::of(wide, high, opaque(&pixels, told.pixel_format, wide * high)?).ok()
}

fn opaque(pixels: &[u8], format: PixelFormat, many: usize) -> Option<Vec<u8>> {
    let apart = match format {
        PixelFormat::RGB24 => 3,
        PixelFormat::L8 => 1,
        _ => return None,
    };
    if pixels.len() != many * apart {
        return None;
    }

    let mut out = Vec::with_capacity(many * 4);
    for one in pixels.chunks_exact(apart) {
        let (red, green, blue) = match one {
            [grey] => (*grey, *grey, *grey),
            [red, green, blue] => (*red, *green, *blue),
            _ => return None,
        };

        out.extend_from_slice(&[red, green, blue, 255]);
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::dotted;

    #[test]
    fn a_tile_is_decoded_at_a_fraction_and_never_larger_than_the_picture() {
        let raw = dotted(1024, 512).written_as("jpg").expect("a jpeg");

        let tile = drawn(&raw, Some(160)).expect("it reads");
        assert!(
            tile.wide <= 256 && tile.wide >= 160,
            "jpeg only shrinks by halves, so a tile of 160 comes off the eighth-size pass at 128 \
             or the quarter-size pass at 256, and the caller shrinks the rest of the way: {}x{}",
            tile.wide,
            tile.high
        );
        assert_eq!(
            tile.wide * 512,
            tile.high * 1024,
            "whatever fraction it picked, the shape has to survive it"
        );

        let whole = drawn(&raw, None).expect("it reads");
        assert_eq!(
            (whole.wide, whole.high),
            (1024, 512),
            "asking for no size hands back the picture the game ships, which is what goes to the \
             clipboard and back into the game"
        );
    }

    #[test]
    fn a_picture_smaller_than_the_tile_is_left_at_its_own_size() {
        let raw = dotted(64, 48).written_as("jpg").expect("a jpeg");
        let held = drawn(&raw, Some(4096)).expect("it reads");

        assert_eq!((held.wide, held.high), (64, 48));
    }

    #[test]
    fn every_pixel_comes_back_opaque_and_in_the_order_the_screen_wants() {
        let raw = dotted(32, 32).written_as("jpg").expect("a jpeg");
        let held = drawn(&raw, None).expect("it reads");

        assert_eq!(held.pixels.len(), 32 * 32 * 4);
        assert!(
            held.pixels.chunks(4).all(|one| one[3] == 255),
            "a jpeg holds no transparency, so a tile that came back part see through would be the \
             checkerboard showing through a picture that has no holes in it"
        );
        assert!(
            held.pixels.chunks(4).any(|one| one[0] != one[2]),
            "a grey tile would mean the channels were read in the wrong order or dropped"
        );
    }

    #[test]
    fn what_is_not_a_jpeg_falls_through_to_the_reader_that_knows_it() {
        assert!(drawn(b"not a jpeg", None).is_none());
        assert!(drawn(&[], Some(64)).is_none());
    }
}
