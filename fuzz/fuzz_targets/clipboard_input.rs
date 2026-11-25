#![no_main]
use libfuzzer_sys::fuzz_target;
use yttraydl::platform::decode_clipboard_units;

fuzz_target!(|data: &[u8]| {
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let decoded = decode_clipboard_units(&units);
    assert_eq!(
        decoded.is_some(),
        units.iter().take(1_000_000).any(|unit| *unit == 0)
    );
});
