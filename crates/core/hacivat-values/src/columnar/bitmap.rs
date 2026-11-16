//! A growable bitmap in `u64` words (Task 6): the validity of a typed column buffer (a set
//! bit is a value, a clear bit a NULL or an exception) and the bits of a `Bool` buffer.

/// Bits in `u64` words, least significant bit first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Bitmap {
    words: Vec<u64>,
    len: usize,
}

impl Bitmap {
    /// An empty bitmap with room for `bits`.
    pub(crate) fn with_capacity(bits: usize) -> Self {
        Bitmap {
            words: Vec::with_capacity(bits.div_ceil(64)),
            len: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn push(&mut self, bit: bool) {
        let (word, shift) = (self.len / 64, self.len % 64);
        if shift == 0 {
            self.words.push(0);
        }
        if bit {
            self.words[word] |= 1 << shift;
        }
        self.len += 1;
    }

    /// Bit `i`.
    ///
    /// # Panics
    /// When `i` is outside the bitmap's words.
    #[inline]
    pub(crate) fn get(&self, i: usize) -> bool {
        (self.words[i / 64] >> (i % 64)) & 1 == 1
    }

    /// The bytes its words hold.
    pub(crate) fn bytes(&self) -> u64 {
        (self.words.len() * size_of::<u64>()) as u64
    }

    pub(crate) fn shrink_to_fit(&mut self) {
        self.words.shrink_to_fit();
    }
}

#[cfg(test)]
mod tests {
    use super::Bitmap;

    #[test]
    fn bits_cross_word_boundaries() {
        let mut b = Bitmap::with_capacity(0);
        for i in 0..130 {
            b.push(i % 3 == 0);
        }
        assert_eq!(b.len(), 130);
        assert!((0..130).all(|i| b.get(i) == (i % 3 == 0)));
        assert_eq!(b.bytes(), 3 * 8);
    }
}
