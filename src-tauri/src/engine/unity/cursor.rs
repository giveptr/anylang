use anyhow::{Result, bail};
use std::str;

pub struct At<'a> {
    pub raw: &'a [u8],
    pub seen: usize,
}

impl<'a> At<'a> {
    pub fn new(raw: &'a [u8]) -> Self {
        Self { raw, seen: 0 }
    }

    pub fn take(&mut self, many: usize) -> Result<&'a [u8]> {
        let end = self.seen + many;
        if end > self.raw.len() {
            bail!("the file ends early");
        }

        let slice = &self.raw[self.seen..end];
        self.seen = end;

        Ok(slice)
    }

    pub fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into()?))
    }

    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into()?))
    }

    pub fn big16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into()?))
    }

    pub fn big32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into()?))
    }

    pub fn big64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into()?))
    }

    pub fn zero_ended(&mut self) -> Result<String> {
        if self.seen > self.raw.len() {
            bail!("the file ends early");
        }

        let start = self.seen;
        while self.seen < self.raw.len() && self.raw[self.seen] != 0 {
            self.seen += 1;
        }

        let out = String::from_utf8_lossy(&self.raw[start..self.seen]).into_owned();
        self.seen += 1;

        Ok(out)
    }

    pub fn align(&mut self, to: usize) {
        self.seen = self.seen.next_multiple_of(to);
    }
}

pub const STEP: usize = 4;

pub fn word_at(body: &[u8], at: usize) -> Option<(&str, usize)> {
    let head = body.get(at..at + STEP)?;
    let wide = u32::from_le_bytes(head.try_into().ok()?) as usize;

    let end = at + STEP + wide;
    let said = str::from_utf8(body.get(at + STEP..end)?).ok()?;

    Some((said, end.next_multiple_of(STEP)))
}

pub fn put_word(out: &mut Vec<u8>, said: &str) {
    out.extend_from_slice(&(said.len() as u32).to_le_bytes());
    out.extend_from_slice(said.as_bytes());

    while !out.len().is_multiple_of(STEP) {
        out.push(0);
    }
}
