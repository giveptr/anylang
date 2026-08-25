use crate::canvas::{Canvas, coarse, smaller};
use libwebp_sys as sys;
use std::mem::MaybeUninit;

fn told(raw: &[u8]) -> Option<(usize, usize)> {
    let mut held = MaybeUninit::<sys::WebPBitstreamFeatures>::zeroed();

    let said = unsafe { sys::WebPGetFeatures(raw.as_ptr(), raw.len(), held.as_mut_ptr()) };
    if said != sys::VP8StatusCode::VP8_STATUS_OK {
        return None;
    }

    let held = unsafe { held.assume_init() };
    if held.has_animation != 0 || held.width < 1 || held.height < 1 {
        return None;
    }

    Some((held.width as usize, held.height as usize))
}

pub fn drawn(raw: &[u8], most: Option<usize>) -> Option<Canvas> {
    let shipped = told(raw)?;
    let (wide, high) = most
        .filter(|&most| coarse(shipped.0.max(shipped.1), most))
        .and_then(|most| smaller(shipped.0, shipped.1, most))
        .unwrap_or(shipped);

    let mut config = sys::WebPDecoderConfig::new().ok()?;
    let mut pixels = vec![0u8; wide.checked_mul(high)?.checked_mul(4)?];
    let stride = i32::try_from(wide.checked_mul(4)?).ok()?;

    config.options.use_scaling = i32::from((wide, high) != shipped);
    config.options.scaled_width = i32::try_from(wide).ok()?;
    config.options.scaled_height = i32::try_from(high).ok()?;
    config.output.colorspace = sys::WEBP_CSP_MODE::MODE_RGBA;
    config.output.is_external_memory = 1;
    config.output.u.RGBA.rgba = pixels.as_mut_ptr();
    config.output.u.RGBA.stride = stride;
    config.output.u.RGBA.size = pixels.len();

    let said = unsafe { sys::WebPDecode(raw.as_ptr(), raw.len(), &mut config) };
    unsafe { sys::WebPFreeDecBuffer(&mut config.output) };

    if said != sys::VP8StatusCode::VP8_STATUS_OK {
        return None;
    }

    Canvas::of(wide, high, pixels).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::dotted;

    #[test]
    fn a_webp_is_read_at_its_own_size_and_at_the_size_that_was_asked_for() {
        let raw = dotted(320, 200).written_as("webp").expect("a webp");

        let whole = drawn(&raw, None).expect("it reads");
        assert_eq!(
            (whole.wide, whole.high),
            (320, 200),
            "asking for no size has to hand back the picture the game ships, because that is what \
             goes to the clipboard and back into the game"
        );
        assert_eq!(whole.pixels.len(), 320 * 200 * 4);

        let tile = drawn(&raw, Some(32)).expect("it reads");
        assert_eq!(
            (tile.wide, tile.high),
            (32, 20),
            "a tile is decoded straight to the size the panel draws it at, which is the whole \
             reason for reaching past the image crate: turning two million pixels into six hundred \
             was most of what a gallery spent its time and the reader's fan on"
        );
        assert_eq!(tile.pixels.len(), 32 * 20 * 4);
    }

    #[test]
    fn a_near_miss_comes_back_whole_so_the_sharper_filter_gets_to_finish_it() {
        let raw = dotted(100, 80).written_as("webp").expect("a webp");
        let held = drawn(&raw, Some(64)).expect("it reads");

        assert_eq!(
            (held.wide, held.high),
            (100, 80),
            "libwebp rescales by box average, which is fine for a tile an eighth the size but \
             soft for a preview barely shrunk, so a near miss is decoded whole and the caller \
             finishes it with the filter the policy in filter_for promises"
        );
    }

    #[test]
    fn what_is_not_a_still_webp_falls_through_to_the_reader_that_knows_it() {
        assert!(
            drawn(b"not a webp at all", None).is_none(),
            "a png, a jpeg or a truncated file has to fall through to the image crate rather than \
             come back as an empty picture"
        );
        assert!(drawn(&[], Some(64)).is_none());
        assert!(drawn(&[0; 4096], None).is_none());
    }

    #[test]
    fn the_colours_survive_the_trip_through_c() {
        let held = dotted(48, 48);
        let raw = held.written_as("webp").expect("a webp");
        let back = drawn(&raw, None).expect("it reads");

        assert_eq!(
            &back.pixels[..8],
            &held.pixels[..8],
            "the reader picks replacement art by eye, so a channel order swapped on the way out of \
             libwebp would show them a blue character under a red sky"
        );
        assert!(
            back.pixels.chunks(4).all(|one| one[3] == 255),
            "an opaque picture may not come back part transparent"
        );
    }
}
