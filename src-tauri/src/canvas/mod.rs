mod jpeg;
mod webp;

use anyhow::{Context, Result, bail};
use image::imageops::FilterType;
use image::{ImageEncoder, RgbaImage};
use std::io::Cursor;
use std::path::Path;
use std::str;
use std::sync::LazyLock;

const WIDEST: u32 = 32768;

static KINDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut out: Vec<&'static str> = image::ImageFormat::all()
        .filter(image::ImageFormat::reading_enabled)
        .flat_map(image::ImageFormat::extensions_str)
        .copied()
        .collect();

    out.sort_unstable();
    out.dedup();

    out
});

pub fn kinds() -> Vec<&'static str> {
    KINDS.clone()
}

pub fn measured(raw: &[u8]) -> Option<(u32, u32)> {
    let held = imagesize::blob_size(raw).ok()?;

    Some((
        u32::try_from(held.width).ok()?,
        u32::try_from(held.height).ok()?,
    ))
}

const DRAWN: &str = "svg";

pub fn kind_of(raw: &[u8]) -> Option<&'static str> {
    if let Ok(held) = image::guess_format(raw) {
        return held.extensions_str().first().copied();
    }

    let ahead = raw.len().min(256);
    match str::from_utf8(&raw[..ahead]).is_ok_and(|head| head.trim_start().starts_with('<'))
        && raw.windows(4).take(ahead).any(|held| held == b"<svg")
    {
        true => Some(DRAWN),
        false => None,
    }
}

const AS_DRAWN: &str = "image/svg+xml";

pub fn shown_as(raw: &[u8]) -> Option<&'static str> {
    use image::ImageFormat::{Bmp, Gif, Ico, Jpeg, Png, WebP};

    match image::guess_format(raw) {
        Ok(held @ (Png | Jpeg | Gif | WebP | Bmp | Ico)) => Some(held.to_mime_type()),
        Ok(_) => None,
        Err(_) => (kind_of(raw) == Some(DRAWN)).then_some(AS_DRAWN),
    }
}

pub fn writes(kind: &str) -> bool {
    known(kind).is_some_and(|held| held.writing_enabled())
}

pub fn reads(kind: &str) -> bool {
    known(kind).is_some_and(|held| held.reading_enabled())
}

pub fn same_format(one: &str, two: &str) -> bool {
    match (known(one), known(two)) {
        (Some(held), Some(other)) => held == other,
        _ => one.eq_ignore_ascii_case(two),
    }
}

fn known(kind: &str) -> Option<image::ImageFormat> {
    image::ImageFormat::all().find(|held| {
        held.extensions_str()
            .iter()
            .any(|one| one.eq_ignore_ascii_case(kind))
    })
}

pub fn is_picture(at: &Path) -> bool {
    at.extension()
        .is_some_and(|held| KINDS.iter().any(|one| held.eq_ignore_ascii_case(one)))
}

pub fn smaller(wide: usize, high: usize, most: usize) -> Option<(usize, usize)> {
    let widest = wide.max(high);
    if most == 0 || widest <= most || widest == 0 {
        return None;
    }

    Some(((wide * most / widest).max(1), (high * most / widest).max(1)))
}

fn straight(raw: &[u8], most: Option<usize>) -> Option<Canvas> {
    match image::guess_format(raw).ok()? {
        image::ImageFormat::WebP => webp::drawn(raw, most),
        image::ImageFormat::Jpeg => jpeg::drawn(raw, most),
        _ => None,
    }
}

fn coarse(widest: usize, most: usize) -> bool {
    widest >= most.saturating_mul(2)
}

fn filter_for(widest: usize, most: usize) -> FilterType {
    match coarse(widest, most) {
        true => FilterType::Triangle,
        false => FilterType::Lanczos3,
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Canvas {
    pub wide: usize,
    pub high: usize,
    pub pixels: Vec<u8>,
}

impl Canvas {
    #[cfg(test)]
    pub fn blank(wide: usize, high: usize) -> Self {
        Self {
            wide,
            high,
            pixels: vec![0; wide * high * 4],
        }
    }

    pub fn of(wide: usize, high: usize, pixels: Vec<u8>) -> Result<Self> {
        let wanted = wide
            .checked_mul(high)
            .and_then(|many| many.checked_mul(4))
            .ok_or_else(|| anyhow::anyhow!("{wide}x{high} is larger than any image could be"))?;

        if pixels.len() != wanted {
            bail!(
                "an image of {wide}x{high} asks for {wanted} bytes and was handed {}",
                pixels.len()
            );
        }

        Ok(Self { wide, high, pixels })
    }

    pub fn read(raw: &[u8]) -> Result<Self> {
        Self::read_within(raw, 0)
    }

    pub fn read_within(raw: &[u8], most: usize) -> Result<Self> {
        let asked = (most > 0).then_some(most);
        let held = match straight(raw, asked) {
            Some(held) => held,
            None => Self::whole(raw)?,
        };

        match smaller(held.wide, held.high, most) {
            Some(_) => held.within(most),
            None => Ok(held),
        }
    }

    fn whole(raw: &[u8]) -> Result<Self> {
        let held = image::load_from_memory(raw)
            .context("this file is not an image this reader can open")?
            .into_rgba8();

        Self::of(
            held.width() as usize,
            held.height() as usize,
            held.into_raw(),
        )
    }

    pub fn png(&self) -> Result<Vec<u8>> {
        let (Ok(wide), Ok(high)) = (u32::try_from(self.wide), u32::try_from(self.high)) else {
            bail!(
                "{}x{} is larger than a png could hold",
                self.wide,
                self.high
            );
        };

        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new_with_quality(
            &mut out,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Adaptive,
        )
        .write_image(&self.pixels, wide, high, image::ExtendedColorType::Rgba8)
        .context("writing the image out as a png")?;

        Ok(out)
    }

    pub fn written_as(&self, kind: &str) -> Result<Vec<u8>> {
        let held = known(kind)
            .filter(|held| held.writing_enabled())
            .ok_or_else(|| anyhow::anyhow!("this build cannot write a {kind} back out"))?;

        if held == image::ImageFormat::Png {
            return self.png();
        }

        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(self.holding()?)
            .write_to(&mut out, held)
            .with_context(|| format!("writing the image out as {kind}"))?;

        Ok(out.into_inner())
    }

    pub fn flipped(&self) -> Self {
        let row = self.wide * 4;
        let mut pixels = Vec::with_capacity(self.pixels.len());

        for line in (0..self.high).rev() {
            pixels.extend_from_slice(&self.pixels[line * row..line * row + row]);
        }

        Self {
            wide: self.wide,
            high: self.high,
            pixels,
        }
    }

    pub fn mirrored(&self) -> Self {
        let mut pixels = Vec::with_capacity(self.pixels.len());

        for line in 0..self.high {
            let row = &self.pixels[line * self.wide * 4..(line + 1) * self.wide * 4];
            for column in (0..self.wide).rev() {
                pixels.extend_from_slice(&row[column * 4..column * 4 + 4]);
            }
        }

        Self {
            wide: self.wide,
            high: self.high,
            pixels,
        }
    }

    pub fn scaled(&self, wide: usize, high: usize) -> Result<Self> {
        self.scaled_with(wide, high, FilterType::Lanczos3)
    }

    fn scaled_with(&self, wide: usize, high: usize, filter: FilterType) -> Result<Self> {
        if wide == self.wide && high == self.high {
            return Ok(self.clone());
        }

        let held = self.holding()?;
        let (Ok(wide), Ok(high)) = (u32::try_from(wide), u32::try_from(high)) else {
            bail!("{wide}x{high} is larger than any image could be");
        };
        if wide == 0 || high == 0 || wide > WIDEST || high > WIDEST {
            bail!("{wide}x{high} is not a size an image can be scaled to");
        }

        let done = image::imageops::resize(&held, wide, high, filter);

        Self::of(wide as usize, high as usize, done.into_raw())
    }

    pub fn within(&self, most: usize) -> Result<Self> {
        let Some((wide, high)) = smaller(self.wide, self.high, most) else {
            return Ok(self.clone());
        };

        self.scaled_with(wide, high, filter_for(self.wide.max(self.high), most))
    }

    pub fn cut(&self, from_x: usize, from_y: usize, wide: usize, high: usize) -> Result<Self> {
        if from_x + wide > self.wide || from_y + high > self.high {
            bail!(
                "a cut of {wide}x{high} at {from_x},{from_y} reaches past an image of {}x{}",
                self.wide,
                self.high
            );
        }

        let mut pixels = Vec::with_capacity(wide * high * 4);
        for line in 0..high {
            let at = ((from_y + line) * self.wide + from_x) * 4;
            pixels.extend_from_slice(&self.pixels[at..at + wide * 4]);
        }

        Self::of(wide, high, pixels)
    }

    pub fn paint(&mut self, from_x: usize, from_y: usize, held: &Canvas) -> Result<()> {
        if from_x + held.wide > self.wide || from_y + held.high > self.high {
            bail!(
                "an image of {}x{} does not fit at {from_x},{from_y} inside one of {}x{}",
                held.wide,
                held.high,
                self.wide,
                self.high
            );
        }

        for line in 0..held.high {
            let at = ((from_y + line) * self.wide + from_x) * 4;
            let held_at = line * held.wide * 4;

            self.pixels[at..at + held.wide * 4]
                .copy_from_slice(&held.pixels[held_at..held_at + held.wide * 4]);
        }

        Ok(())
    }

    fn holding(&self) -> Result<RgbaImage> {
        let (Ok(wide), Ok(high)) = (u32::try_from(self.wide), u32::try_from(self.high)) else {
            bail!(
                "{}x{} is larger than any image could be",
                self.wide,
                self.high
            );
        };

        RgbaImage::from_raw(wide, high, self.pixels.clone()).ok_or_else(|| {
            anyhow::anyhow!("an image of {wide}x{high} does not hold its own pixels")
        })
    }
}

#[cfg(test)]
pub fn dotted(wide: usize, high: usize) -> Canvas {
    let mut held = Canvas::blank(wide, high);

    for line in 0..high {
        for column in 0..wide {
            let at = (line * wide + column) * 4;
            held.pixels[at] = (column * 8) as u8;
            held.pixels[at + 1] = (line * 8) as u8;
            held.pixels[at + 2] = 7;
            held.pixels[at + 3] = 255;
        }
    }

    held
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn a_picture_written_back_keeps_the_transparency_every_format_can_hold() {
        let mut held = Canvas::blank(4, 4);
        for (which, byte) in held.pixels.iter_mut().enumerate() {
            *byte = match which % 4 {
                3 => 0,
                _ => 200,
            };
        }

        let flat = [image::ImageFormat::Jpeg];

        for kind in kinds().into_iter().filter(|kind| writes(kind)) {
            let format = known(kind).expect("a format this build named itself");
            let body = held
                .written_as(kind)
                .unwrap_or_else(|why| panic!("{kind} is a format this build writes: {why:#}"));

            let back = image::load_from_memory_with_format(&body, format)
                .unwrap_or_else(|why| panic!("{kind} reads back: {why}"))
                .into_rgba8();

            if flat.contains(&format) {
                continue;
            }

            assert!(
                back.pixels().all(|one| one.0[3] == 0),
                "a game that ships {kind} sprites keeps their transparency, and writing one back \
                 opaque paints a solid box over the character"
            );
        }
    }

    #[test]
    fn a_picture_gives_up_its_size_from_the_head_alone() {
        let held = dotted(40, 24);

        for kind in ["png", "jpg", "gif", "bmp", "webp"] {
            let whole = held.written_as(kind).expect(kind);
            let head = &whole[..whole.len().min(64)];

            assert_eq!(
                measured(head),
                Some((40, 24)),
                "listing a game of twenty thousand pictures means reading a head each, not a \
                 whole file each: {kind} has to answer from {} byte(s)",
                head.len()
            );
        }

        assert_eq!(measured(b"not a picture"), None);
        assert_eq!(measured(&[]), None);
    }

    #[test]
    fn what_a_file_holds_is_read_from_its_bytes_and_never_from_its_name() {
        let held = dotted(8, 4);
        let webp = held.written_as("webp").expect("a webp");

        assert_eq!(
            kind_of(&webp),
            Some("webp"),
            "one real game ships webp bytes under png names, and Ren'Py loads them because it \
             sniffs: a reader that trusted the ending would tell the translator the wrong format \
             and write the wrong one back"
        );
        assert_eq!(measured(&webp), Some((8, 4)));
    }

    #[test]
    fn the_endings_this_build_can_open_are_the_ones_it_was_built_with() {
        let held = kinds();

        assert!(
            held.contains(&"png"),
            "png is what every preview is drawn as, so a build that cannot read one back is a \
             build where nothing shows"
        );
        assert!(
            held.len() >= 2,
            "the reader picks a replacement from disk, and a list of one ending would turn away \
             files this build opens perfectly well: {held:?}"
        );
        assert_eq!(
            held.iter().collect::<BTreeSet<_>>().len(),
            held.len(),
            "the list reaches a file dialog, and a doubled ending shows up twice on the screen"
        );
    }

    #[test]
    fn a_file_is_a_picture_only_when_its_ending_is_one_this_build_reads() {
        assert!(is_picture(Path::new("art/day.PNG")));
        assert!(!is_picture(Path::new("art/day.rpy")));
        assert!(
            !is_picture(Path::new("art/day")),
            "a file with no ending at all is not one this reader would open as a picture"
        );
    }

    #[test]
    fn an_image_written_out_as_a_png_reads_back_pixel_for_pixel() {
        let held = dotted(8, 5);
        let back = Canvas::read(&held.png().expect("a png")).expect("it reads back");

        assert_eq!(
            (back.wide, back.high),
            (8, 5),
            "the preview a reader sees has to be the image that is in the game"
        );
        assert_eq!(
            back.pixels, held.pixels,
            "a png keeps every channel it was handed, so a round trip that changed one would mean \
             the reader is picking a colour the game never had"
        );
    }

    #[test]
    fn flipping_an_image_twice_gives_the_image_back() {
        let held = dotted(4, 3);

        assert_eq!(
            held.flipped().flipped().pixels,
            held.pixels,
            "Unity keeps its rows bottom up, so every read flips and every write flips back: a \
             flip that is not its own opposite would land the picture upside down in the game"
        );
        assert_ne!(
            held.flipped().pixels,
            held.pixels,
            "and the flip has to actually move the rows"
        );
    }

    #[test]
    fn mirroring_an_image_turns_it_around_the_other_way_than_flipping_does() {
        let held = dotted(4, 3);

        assert_eq!(
            held.mirrored().mirrored().pixels,
            held.pixels,
            "a sprite packer stores some sprites mirrored, and the same turn takes it out and puts \
             it back: one that is not its own opposite would land the picture the wrong way round \
             in the game"
        );
        assert_ne!(held.mirrored().pixels, held.pixels);
        assert_ne!(
            held.mirrored().pixels,
            held.flipped().pixels,
            "left to right and top to bottom are two different turns, and reading one as the \
             other shows the reader a picture nobody drew"
        );

        let piece = held.mirrored().cut(0, 0, 1, 1).expect("its first pixel");
        assert_eq!(
            piece.pixels,
            held.cut(3, 0, 1, 1).expect("its last pixel").pixels,
            "the column that was last has to be the column that comes first"
        );
    }

    #[test]
    fn a_cut_holds_the_very_pixels_that_sat_in_that_corner() {
        let held = dotted(8, 8);
        let piece = held.cut(2, 3, 3, 2).expect("a cut inside the image");

        assert_eq!((piece.wide, piece.high), (3, 2));
        for line in 0..2 {
            for column in 0..3 {
                let there = (line * 3 + column) * 4;
                let here = ((line + 3) * 8 + column + 2) * 4;

                assert_eq!(
                    piece.pixels[there..there + 4],
                    held.pixels[here..here + 4],
                    "a sprite is named by where it sits in its atlas, so a cut that drifts by a \
                     row hands the reader somebody else's picture"
                );
            }
        }

        assert!(
            held.cut(6, 0, 4, 1).is_err(),
            "a cut reaching past the atlas is a rect this reader read wrongly, and guessing at it \
             would show a picture nobody drew"
        );
    }

    #[test]
    fn painting_a_piece_in_leaves_every_pixel_around_it_alone() {
        let mut held = dotted(8, 8);
        let was = held.clone();
        let piece = Canvas::of(2, 2, vec![9; 16]).expect("a small piece");

        held.paint(5, 6, &piece).expect("it fits");

        for line in 0..8 {
            for column in 0..8 {
                let at = (line * 8 + column) * 4;
                let inside = (5..7).contains(&column) && (6..8).contains(&line);

                match inside {
                    true => assert_eq!(held.pixels[at..at + 4], [9, 9, 9, 9]),
                    false => assert_eq!(
                        held.pixels[at..at + 4],
                        was.pixels[at..at + 4],
                        "one sprite of an atlas is replaced at a time, and the other forty five in \
                         the same strip belong to the game"
                    ),
                }
            }
        }

        assert!(
            held.paint(7, 7, &piece).is_err(),
            "a piece hanging over the edge would wrap onto the row below"
        );
    }

    #[test]
    fn a_thumbnail_keeps_the_shape_of_what_it_stands_for() {
        let wide = dotted(400, 100).within(64).expect("a thumbnail");
        assert_eq!((wide.wide, wide.high), (64, 16));

        let small = dotted(20, 10).within(64).expect("a thumbnail");
        assert_eq!(
            (small.wide, small.high),
            (20, 10),
            "an image already smaller than the tile is left as it is: growing it only blurs it"
        );

        let thin = dotted(15600, 300).within(96).expect("a thumbnail");
        assert_eq!(
            (thin.wide, thin.high),
            (96, 1),
            "an atlas strip is fifty times wider than it is tall, and a tile has to keep that or \
             the reader cannot tell one strip from another"
        );
    }

    #[test]
    fn a_tile_is_averaged_down_and_only_a_near_miss_is_worth_resampling_sharply() {
        assert_eq!(
            filter_for(1920, 160),
            FilterType::Triangle,
            "a gallery of a game draws thousands of 1080p pictures down to a 160px tile, and \
             Lanczos3 over a reduction that large costs three times as much for a tile nobody can \
             tell apart"
        );
        assert_eq!(
            filter_for(800, 640),
            FilterType::Lanczos3,
            "a preview that is barely shrunk is looked at closely, so it is worth the sharper \
             filter"
        );
        assert_eq!(filter_for(1280, 640), FilterType::Triangle);
    }

    #[test]
    fn a_tile_of_one_colour_comes_out_that_colour() {
        let mut held = Canvas::blank(1920, 1080);
        for (which, byte) in held.pixels.iter_mut().enumerate() {
            *byte = match which % 4 {
                0 => 30,
                1 => 90,
                2 => 200,
                _ => 255,
            };
        }

        let tile = held.within(160).expect("a tile");
        assert_eq!((tile.wide, tile.high), (160, 90));
        assert!(
            tile.pixels.chunks(4).all(|one| one == [30, 90, 200, 255]),
            "a filter that averaged in the edge of the picture, or clipped a channel, would show \
             the reader a colour the game never drew"
        );
    }

    #[test]
    fn a_canvas_the_bytes_cannot_fill_is_refused_and_an_empty_one_still_scales() {
        assert!(Canvas::of(2, 2, vec![0; 8]).is_err());
        assert!(Canvas::read(b"not an image at all").is_err());
        assert!(Canvas::blank(0, 0).within(64).is_ok());
    }
}
