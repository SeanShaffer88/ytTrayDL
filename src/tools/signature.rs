//! Fail-closed verifier for yt-dlp's pinned RSA OpenPGP release signatures.
//!
//! Deliberately supports only binary-document v4 RSA/SHA-512 detached
//! signatures, the format published by yt-dlp. Any new algorithm or packet
//! format requires a reviewed app update rather than silently bypassing checks.
use ring::signature::{RsaPublicKeyComponents, RSA_PKCS1_2048_8192_SHA512};
use sha2::{Digest, Sha512};

// The primary release key from yt-dlp/public.key, fingerprint
// AC0C BBE6 848D 6A87 3464 AF4E 57CF 6593 3B5A 7581, RSA exponent 65537.
const MODULUS: &[u8] = include_bytes!("yt-dlp-release-key.rsa");
const EXPONENT: &[u8] = &[1, 0, 1];
const MAX_SUMS: usize = 64 * 1024;
const MAX_SIGNATURE: usize = 8 * 1024;

fn u16_at(bytes: &[u8], offset: usize) -> Option<usize> {
    Some(u16::from_be_bytes(bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?) as usize)
}

/// Parse exactly one bounded OpenPGP signature packet (old or new definite
/// length); reject partial/indeterminate lengths and trailing packets.
fn signature_packet(bytes: &[u8]) -> Option<&[u8]> {
    let ctb = *bytes.first()?;
    if ctb & 0x80 == 0 {
        return None;
    }
    let (header, length) = if ctb & 0x40 == 0 {
        if (ctb >> 2) & 0x0f != 2 {
            return None;
        }
        match ctb & 3 {
            0 => (2, *bytes.get(1)? as usize),
            1 => (3, u16_at(bytes, 1)?),
            2 => (
                5,
                u32::from_be_bytes(bytes.get(1..5)?.try_into().ok()?) as usize,
            ),
            _ => return None,
        }
    } else {
        if ctb & 0x3f != 2 {
            return None;
        }
        let first = *bytes.get(1)? as usize;
        match first {
            0..=191 => (2, first),
            192..=223 => (3, ((first - 192) << 8) + *bytes.get(2)? as usize + 192),
            255 => (
                6,
                u32::from_be_bytes(bytes.get(2..6)?.try_into().ok()?) as usize,
            ),
            _ => return None,
        }
    };
    if header + length != bytes.len() {
        return None;
    }
    bytes.get(header..)
}

/// Reject unknown critical hashed subpackets. Their semantics cannot be
/// ignored safely even when the mathematical signature is valid.
fn supported_hashed_subpackets(mut bytes: &[u8]) -> bool {
    while !bytes.is_empty() {
        let first = bytes[0] as usize;
        let (header, size) = match first {
            0 => return false,
            1..=191 => (1, first),
            192..=223 if bytes.len() >= 2 => (2, ((first - 192) << 8) + bytes[1] as usize + 192),
            255 if bytes.len() >= 5 => (
                5,
                u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize,
            ),
            _ => return false,
        };
        if size == 0 || header + size > bytes.len() {
            return false;
        }
        let kind = bytes[header];
        // Creation time and issuer fingerprint have no extra semantics to
        // enforce for our independently pinned primary key.
        if kind & 0x80 != 0 && !matches!(kind & 0x7f, 2 | 33) {
            return false;
        }
        bytes = &bytes[header + size..];
    }
    true
}

/// Validate the detached signature and return the unique SHA-256 entry for the
/// executable. The caller must also compare this with the downloaded file and
/// GitHub release digest; a valid signature alone does not verify the binary.
pub(super) fn verify(sums: &[u8], signature: &[u8], asset_name: &str) -> Result<String, String> {
    verify_with_key(sums, signature, asset_name, MODULUS, EXPONENT)
}

fn verify_with_key(
    sums: &[u8],
    signature: &[u8],
    asset_name: &str,
    modulus: &[u8],
    exponent: &[u8],
) -> Result<String, String> {
    let invalid = || "Invalid or unsupported yt-dlp release signature".to_string();
    if sums.is_empty()
        || sums.len() > MAX_SUMS
        || signature.is_empty()
        || signature.len() > MAX_SIGNATURE
    {
        return Err(invalid());
    }
    let packet = signature_packet(signature).ok_or_else(invalid)?;
    if packet.get(..4) != Some(&[4, 0, 1, 10]) {
        return Err(invalid());
    }
    let hashed_len = u16_at(packet, 4).ok_or_else(invalid)?;
    let hashed_end = 6usize
        .checked_add(hashed_len)
        .filter(|end| *end <= packet.len())
        .ok_or_else(invalid)?;
    if !supported_hashed_subpackets(&packet[6..hashed_end]) {
        return Err(invalid());
    }
    let unhashed_len = u16_at(packet, hashed_end).ok_or_else(invalid)?;
    let hash_start = hashed_end
        .checked_add(2)
        .and_then(|v| v.checked_add(unhashed_len))
        .ok_or_else(invalid)?;
    let hash_prefix = packet.get(hash_start..hash_start + 2).ok_or_else(invalid)?;
    let mpi_start = hash_start + 2;
    let bits = u16_at(packet, mpi_start).ok_or_else(invalid)?;
    let mpi = packet.get(mpi_start + 2..).ok_or_else(invalid)?;
    if bits == 0
        || bits > modulus.len() * 8
        || mpi.len() != bits.div_ceil(8)
        || mpi
            .first()
            .is_none_or(|v| *v == 0 || (8 - v.leading_zeros() as usize) != ((bits - 1) % 8 + 1))
    {
        return Err(invalid());
    }
    let mut signed = Vec::with_capacity(sums.len() + hashed_end + 6);
    signed.extend_from_slice(sums);
    signed.extend_from_slice(&packet[..hashed_end]);
    signed.extend_from_slice(&[4, 0xff]);
    signed.extend_from_slice(&(hashed_end as u32).to_be_bytes());
    if Sha512::digest(&signed)[..2] != *hash_prefix {
        return Err(invalid());
    }
    let mut padded = vec![0; modulus.len()];
    let offset = padded.len() - mpi.len();
    padded[offset..].copy_from_slice(mpi);
    RsaPublicKeyComponents {
        n: modulus,
        e: exponent,
    }
    .verify(&RSA_PKCS1_2048_8192_SHA512, &signed, &padded)
    .map_err(|_| invalid())?;
    let text = std::str::from_utf8(sums).map_err(|_| invalid())?;
    let mut found = None;
    for line in text.lines() {
        let Some((hash, name)) = line.split_once("  ").or_else(|| line.split_once(" *")) else {
            continue;
        };
        if name == asset_name {
            if found.is_some() || !super::valid_digest(hash) {
                return Err(invalid());
            }
            found = Some(hash.to_ascii_lowercase());
        }
    }
    found.ok_or_else(|| "Signed checksum does not uniquely name the yt-dlp executable".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    const SUMS: &[u8] = include_bytes!("../../tests/fixtures/yt-dlp-signature/SHA2-256SUMS");
    const SIG: &[u8] = include_bytes!("../../tests/fixtures/yt-dlp-signature/SHA2-256SUMS.sig");
    #[test]
    fn verifies_upstream_signature_and_rejects_missing_or_modified_data() {
        assert_eq!(
            verify(SUMS, SIG, "yt-dlp.exe").unwrap(),
            "622ed48023cac1daa52a3bf3a6ac38f03514de214b5f0fa069389fd0f6d17e5c"
        );
        assert!(verify(SUMS, &[], "yt-dlp.exe").is_err());
        assert!(verify(SUMS, SIG, "missing.exe").is_err());
        let mut bad_sums = SUMS.to_vec();
        bad_sums[0] ^= 1;
        assert!(verify(&bad_sums, SIG, "yt-dlp.exe").is_err());
        let mut bad_sig = SIG.to_vec();
        *bad_sig.last_mut().unwrap() ^= 1;
        assert!(verify(SUMS, &bad_sig, "yt-dlp.exe").is_err());
        bad_sig = SIG.to_vec();
        bad_sig.push(0);
        assert!(verify(SUMS, &bad_sig, "yt-dlp.exe").is_err());
        assert!(verify_with_key(SUMS, SIG, "yt-dlp.exe", &[1; 512], EXPONENT).is_err());
    }
}
