// https://github.com/KirillKryukov/naf/blob/master/NAFv2.pdf

// And 2bit encoding... SIMD
// https://github.com/Daniel-Liu-c0deb0t/cute-nucleotides

// No for loops in const fns
const fn acceptable_bytes_2bit() -> [u8; 256] {
    let mut acceptable_bytes = [0; 256];
    let ok: [u8; 8] = *b"ATCGatcg";
    let mut i = 0;
    while i < ok.len() {
        acceptable_bytes[ok[i] as usize] = 1;
        i = i + 1;
    }

    acceptable_bytes
    
}

// No for loops in const fns
const fn acceptable_bytes_4bit() -> [u8; 256] {
    let mut acceptable_bytes = [0; 256];
    let ok: [u8; 33] = *b"AaCcGgTtUuRrYySsWwKkMmBbDdHhVvNn-";
    let mut i = 0;
    while i < ok.len() {
        acceptable_bytes[ok[i] as usize] = 1;
        i = i + 1;
    }

    acceptable_bytes
    
}

pub fn can_encode_2bit(seq: &[u8]) -> bool {
    let acceptable_bytes = acceptable_bytes_2bit();
    seq.iter().all(|&b| acceptable_bytes[b as usize] == 1)
}

pub fn can_encode_4bit(seq: &[u8]) -> bool {
    let acceptable_bytes = acceptable_bytes_4bit();
    seq.iter().all(|&b| acceptable_bytes[b as usize] == 1)
}


#[cfg(tests)]
mod tests {
    use super::*;

    #[test]
    fn test_can_encode_2bit() {
        assert!(can_encode_2bit(b"ATCG"));
        assert!(can_encode_2bit(b"atcg"));
        assert!(!can_encode_2bit(b"ATCGN"));
        assert!(!can_encode_2bit(b"atcgn"));
    }

    #[test]
    fn test_can_encode_4bit() {
        assert!(can_encode_4bit(b"ACGTURYSWKMBDHVN-"));
        assert!(can_encode_4bit(b"acgturyswkmbdhvn-"));
        assert!(!can_encode_4bit(b"ACGTURYSWKMBDHjuyteeartVN-?"));
        assert!(!can_encode_4bit(b"acgturyswkmbdhvfdsafadsfdsan-?"));
    }

}