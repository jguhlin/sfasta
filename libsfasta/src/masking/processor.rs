// Masking is just stored as seqloc block

use crate::data_types::*;

// TODO: Benchmark. Use numeric-array? SIMD somehow?
pub fn contains_lowercase(seq: &[u8]) -> bool {
    seq.iter().any(|x| x.is_ascii_lowercase())
}

pub fn find_lowercase_range(block: u32, seq: &[u8]) -> Vec<Loc> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut end = 0;
    let mut in_range = false;
    for (i, x) in seq.iter().enumerate() {
        if x.is_ascii_lowercase() {
            if !in_range {
                start = i;
                in_range = true;
            }
            end = i;
        } else if in_range {
            ranges.push(Loc::new(block, start as u32, end as u32));
            in_range = false;
        }
    }

    if in_range {
        ranges.push(Loc::new(block, start as u32, end as u32));
    }
    ranges
}

pub fn apply_masking(seq: &mut [u8], ranges: &[Loc]) {
    for range in ranges {
        for i in range.start..=range.end {
            seq[i as usize].make_ascii_lowercase();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contains_lowercase() {
        assert!(contains_lowercase(b"ATGCatgc"));
        assert!(!contains_lowercase(b"ATGCATGC"));
    }

    #[test]
    fn test_find_lowercase_range() {
        let mut seq = b"ATGCatgc".to_vec();
        let ranges = find_lowercase_range(0, &seq);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, 4);
        assert_eq!(ranges[0].end, 7);
        apply_masking(&mut seq, &ranges);
        assert_eq!(seq, b"ATGCatgc");
    }

    #[test]
    fn test_apply_masking() {
        let seq = b"ATGCATGC".to_vec();
        let ranges = vec![Loc::new(0, 4, 7)];
        let mut seq2 = seq.clone();
        apply_masking(&mut seq2, &ranges);
        assert_eq!(seq2, b"ATGCatgc");
    }
}