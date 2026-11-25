#![no_main]
use libfuzzer_sys::fuzz_target;

// Compile the same verifier source used by provisioning; no public fuzz-only
// API or alternate parser is introduced into the application.
#[path = "../../src/tools/signature.rs"]
mod signature;

// Four bytes of signature length, then signature bytes followed by sums bytes.
// The public upstream fixture in the seed corpus reaches the valid path.
fuzz_target!(|data: &[u8]| {
    let Some(prefix) = data.get(..4) else { return };
    let length = u32::from_be_bytes(prefix.try_into().unwrap()) as usize;
    let Some(rest) = data.get(4..) else { return };
    if length > rest.len() {
        return;
    }
    let (sig, sums) = rest.split_at(length);
    if let Ok(digest) = signature::verify(sums, sig, "yt-dlp.exe") {
        assert!(signature::valid_digest(&digest));
        assert_eq!(digest, digest.to_ascii_lowercase());
        let mut changed = sums.to_vec();
        changed[0] ^= 1; // A valid signed checksum file is necessarily nonempty.
        assert!(signature::verify(&changed, sig, "yt-dlp.exe").is_err());
    }
});
