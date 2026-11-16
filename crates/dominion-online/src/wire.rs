//! Big-endian primitives used by official client 2.3.7.
use crate::{Error, Result};

pub const MAX_COLLECTION: usize = 65_536;
pub const MAX_STRING_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SAFE_LONG: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone)]
pub struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    pub fn position(&self) -> usize {
        self.offset
    }
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }
    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        if self.remaining() < N {
            return Err(Error::Protocol(format!(
                "truncated field at byte {}: need {N}, have {}",
                self.offset,
                self.remaining()
            )));
        }
        let value = self.bytes[self.offset..self.offset + N]
            .try_into()
            .expect("checked field length");
        self.offset += N;
        Ok(value)
    }
    pub fn int(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take()?))
    }
    pub fn long(&mut self) -> Result<i64> {
        let value = i64::from_be_bytes(self.take()?);
        if !(-MAX_SAFE_LONG..=MAX_SAFE_LONG).contains(&value) {
            return Err(Error::Protocol(
                "long exceeds JavaScript safe integer range".into(),
            ));
        }
        Ok(value)
    }
    pub fn double(&mut self) -> Result<f64> {
        Ok(f64::from_be_bytes(self.take()?))
    }
    pub fn boolean(&mut self) -> Result<bool> {
        match self.take::<1>()?[0] {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(Error::Protocol(format!("invalid boolean {value}"))),
        }
    }
    pub fn count(&mut self) -> Result<usize> {
        let count = self.int()?;
        if count < 0 || count as usize > MAX_COLLECTION {
            return Err(Error::Protocol(format!(
                "invalid collection length {count}"
            )));
        }
        let count = count as usize;
        if count > self.remaining() {
            return Err(Error::Protocol(format!(
                "collection length {count} exceeds remaining bytes {}",
                self.remaining()
            )));
        }
        Ok(count)
    }
    pub fn string(&mut self) -> Result<String> {
        let length = self.int()?;
        if length < 0 || length as usize > MAX_STRING_BYTES || length as usize > self.remaining() {
            return Err(Error::Protocol(format!(
                "invalid string byte length {length}"
            )));
        }
        let end = self.offset + length as usize;
        let value = std::str::from_utf8(&self.bytes[self.offset..end])
            .map_err(|_| Error::Protocol("invalid UTF-8 string".into()))?
            .to_owned();
        self.offset = end;
        Ok(value)
    }
    pub fn ints(&mut self) -> Result<Vec<i32>> {
        let count = self.count()?;
        if count > self.remaining() / 4 {
            return Err(Error::Protocol("truncated integer array".into()));
        }
        (0..count).map(|_| self.int()).collect()
    }
    pub fn finish(&self) -> Result<()> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(Error::Protocol(format!(
                "{} trailing bytes",
                self.remaining()
            )))
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Writer {
    bytes: Vec<u8>,
}
impl Writer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn int(&mut self, value: i32) {
        self.bytes.extend(value.to_be_bytes());
    }
    /// Panics for values the JavaScript peer cannot represent exactly.
    pub fn long(&mut self, value: i64) {
        assert!(
            (-MAX_SAFE_LONG..=MAX_SAFE_LONG).contains(&value),
            "unsafe JavaScript long"
        );
        self.bytes.extend(value.to_be_bytes());
    }
    pub fn double(&mut self, value: f64) {
        self.bytes.extend(value.to_be_bytes());
    }
    pub fn boolean(&mut self, value: bool) {
        self.bytes.push(u8::from(value));
    }
    pub fn string(&mut self, value: &str) {
        assert!(value.len() <= MAX_STRING_BYTES, "oversized wire string");
        self.int(value.len() as i32);
        self.bytes.extend(value.as_bytes());
    }
    pub fn ints(&mut self, values: &[i32]) {
        assert!(values.len() <= MAX_COLLECTION, "oversized wire array");
        self.int(values.len() as i32);
        for &value in values {
            self.int(value);
        }
    }
    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn primitive_fixture_roundtrip_and_endianness() {
        let mut w = Writer::new();
        w.int(-42);
        w.long(-MAX_SAFE_LONG);
        w.double(1.25);
        w.boolean(true);
        w.string("Dominion ☃");
        w.ints(&[0, 2, 5]);
        let bytes = w.finish();
        assert_eq!(&bytes[..4], &[255, 255, 255, 214]);
        let mut r = Reader::new(&bytes);
        assert_eq!(r.int().unwrap(), -42);
        assert_eq!(r.long().unwrap(), -MAX_SAFE_LONG);
        assert_eq!(r.double().unwrap(), 1.25);
        assert!(r.boolean().unwrap());
        assert_eq!(r.string().unwrap(), "Dominion ☃");
        assert_eq!(r.ints().unwrap(), [0, 2, 5]);
        r.finish().unwrap();
    }
    #[test]
    fn rejects_malformed_fields() {
        assert!(Reader::new(&[0; 3]).int().is_err());
        assert!(Reader::new(&[2]).boolean().is_err());
        assert!(Reader::new(&i64::MAX.to_be_bytes()).long().is_err());
        assert!(Reader::new(&(-1_i32).to_be_bytes()).string().is_err());
        assert!(Reader::new(&[0, 0, 0, 1, 255]).string().is_err());
        assert!(Reader::new(&[0, 0, 0, 2, 0, 0, 0, 1]).ints().is_err());
        assert!(Reader::new(&[127, 255, 255, 255]).count().is_err());
        assert!(Reader::new(&[0]).finish().is_err());
    }
}
