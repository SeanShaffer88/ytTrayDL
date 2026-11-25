//! Tiny deterministic property input generator for offline regression tests.
//! Failures report their seed and case number; no automatic shrinking is done.

pub struct Generator(u64);
impl Generator {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    pub fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 32) as u32
    }
    pub fn identifier(&mut self) -> String {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
        let len = (self.next_u32() % 32 + 1) as usize;
        (0..len)
            .map(|_| ALPHABET[self.next_u32() as usize % ALPHABET.len()] as char)
            .collect()
    }
}
