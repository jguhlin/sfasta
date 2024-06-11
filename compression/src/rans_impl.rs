// This is me experimenting. htscodecs has rANS already, should use that

// this is not working as of yet

use rans::RansDecoderMulti;
use rans::{
    byte_decoder::{ByteRansDecSymbol, ByteRansDecoder},
    byte_encoder::{ByteRansEncSymbol, ByteRansEncoder},
    RansDecSymbol, RansDecoder, RansEncSymbol, RansEncoder,
    RansEncoderMulti,
};

use std::collections::HashMap;

/// Calculate frequencies
pub fn calculate_frequencies<T>(data: &[T]) -> HashMap<T, u32>
where
    T: Eq + Copy + std::hash::Hash,
{
    let mut freqs = HashMap::new();
    for &byte in data {
        *freqs.entry(byte).or_insert(0) += 1;
    }
    freqs
}

// Convert to cumulative frequencies
// Cumulative first, then frequency
pub fn to_cumulative_frequencies<T>(freqs: &HashMap<T, u32>) -> HashMap<T, (u32, u32)> 
where
    T: Eq + Copy + std::hash::Hash,

{
    let mut cumul = 0;
    let mut cumul_freqs = HashMap::new();
    for (&byte, &freq) in freqs.iter() {
        cumul_freqs.insert(byte, (cumul, freq));
        cumul += freq;
    }
    cumul_freqs
}

// Normalize
pub fn normalize_frequencies<T>(freqs: &mut HashMap<T, (u32, u32)>)
where
    T: Eq + Copy + std::hash::Hash,

{
    let total_freq = freqs.values().map(|&(_, freq)| freq).sum::<u32>();
    for (_, &mut (_, ref mut freq)) in freqs.iter_mut() {
        *freq = (*freq as f64 / total_freq as f64 * 0x10000 as f64) as u32;
    }
}

pub fn calculate_scale_bits<T>(freqs: &HashMap<T, (u32, u32)>) -> u32 {
    // Calc how many bits we need given the total number of symbols and frequencies
    let total_freq = freqs.values().map(|&(_, freq)| freq).sum::<u32>();
    let mut scale_bits = 32 - total_freq.leading_zeros();

    if scale_bits > 16 {
        scale_bits = 16;
    }
    
    scale_bits
}

// Generate the encoding symbols
pub fn generate_encoding_symbols<T>(freqs: &HashMap<T, (u32, u32)>) -> (u32, Vec<ByteRansEncSymbol>) 
where 
    T: Eq + Copy + std::hash::Hash,
{
    let scale_bits = calculate_scale_bits(freqs);
    println!("Scale bits: {}", scale_bits);


    let mut symbols = Vec::new();
    for (_, &(cumul, freq)) in freqs.iter() {
        symbols.push(ByteRansEncSymbol::new(cumul, freq, scale_bits));
    }
    (scale_bits, symbols)
}

// Generate the decoding symbols
pub fn generate_decoding_symbols<T>(freqs: &HashMap<T, (u32, u32)>) -> (u32, Vec<ByteRansDecSymbol>)
where 
    T: Eq + Copy + std::hash::Hash,
{
    let scale_bits = calculate_scale_bits(freqs);

    let mut symbols = Vec::new();
    for (&byte, &(cumul, freq)) in freqs.iter() {
        symbols.push(ByteRansDecSymbol::new(cumul, freq));
    }
    (scale_bits, symbols)
}

pub fn encode<T>(data: &[T], freqs: &HashMap<T, (u32, u32)>) -> Vec<u8>
where
    T: Eq + Copy + std::hash::Hash + Into<usize>,
{
    assert!(data.len() <= 1024);

    let (scale_bits, symbols) = generate_encoding_symbols(freqs);
    println!("Data length: {}", data.len());
    let mut encoder = ByteRansEncoder::new(data.len());
    for &byte in data {
        encoder.put(&symbols[byte.into()]);
    }
    encoder.flush();
    encoder.data().to_owned()

}
