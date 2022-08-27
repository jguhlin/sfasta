use bitvec::prelude::*;

// 32-bit masking language
// Goal is to create masking language that fits in 32-bit words (so we can perform bitpacking)
// If masking isn't needed, the Seqloc should be "None" so it never ends up here

// Each command should be "4" bits...
// With length and whatnot being up to 16bits
//

// [ 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0]
//   0 0 0 0  // PASS
//   1 0 0 0 x x x x // Skip ahead 1/2 byte len x = u4 (0 - 15)
//   0 1 0 0 x x x x x x x x // Skip-ahead 1 byte len x = u8
//   0 1 0 1 x x x x x x x x x x x x x x x x // Skip ahead 2 bytes len x = u16
//   0 0 1 0 x x x x x x x x x x x x x x x x x x x x // Skip ahead u20
//   0 0 1 1 x x x x x x x x x x x x x x x x x x x x x x x x // Skip ahead u24
//   1 0 0 1 x x x x // Skip ahead 1/2 byte len x = u4 (0 - 15)
//   0 1 1 0 x x x x x x x x // Mask 1 byte len x = u8
//   0 1 1 1 x x x x x x x x x x x x x x x x // Mask 2 byte len x = u 16
//   1 1 0 0 x x x x x x x x x x x x x x x x x x x x // Mask u20
//   1 1 0 1 x x x x x x x x x x x x x x x x x x x x x x x x // Mask u24
//   1 1 1 1  // STOP statement (No more masking, ignore the rest of the u32)
// [ 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0]

// Goal is to fit as many commands into a u32 as possible

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Ml32bit {
    Pass,            // Not a command. When reached, ignore the rest of the u32
    SkipAheadu4(u8), // Stored as a u8, but must fit into a u4
    SkipAheadu8(u8),
    SkipAheadu16(u16),
    SkipAheadu20(u32),
    SkipAheadu24(u32),
    Masku4(u8), // Stored as a u8, but must fit into a u4
    Masku8(u8),
    Masku16(u16),
    Masku20(u32),
    Masku24(u32),
    Stop, // Complete stop
}

impl Ml32bit {
    pub fn bitsize(&self) -> usize {
        match self {
            Ml32bit::Pass => 4,
            Ml32bit::SkipAheadu4(_) => 8,
            Ml32bit::SkipAheadu8(_) => 12,
            Ml32bit::SkipAheadu16(_) => 20,
            Ml32bit::SkipAheadu20(_) => 24,
            Ml32bit::SkipAheadu24(_) => 28,
            Ml32bit::Masku4(_) => 8,
            Ml32bit::Masku8(_) => 12,
            Ml32bit::Masku16(_) => 20,
            Ml32bit::Masku20(_) => 24,
            Ml32bit::Masku24(_) => 28,
            Ml32bit::Stop => 4,
        }
    }
}

pub fn parse_ml32bit(ml: u32) -> Vec<Ml32bit> {
    let ml = ml.view_bits::<Lsb0>();

    let mut commands = Vec::new();
    let mut i = 0;

    loop {
        let command = ml[i..i + 4].load::<u8>();
        match command {
            0b0000 => {
                commands.push(Ml32bit::Pass);
                break;
            }
            0b1000 => {
                let len = ml[i + 4..i + 8].load::<u8>();
                commands.push(Ml32bit::SkipAheadu4(len));
                i += 8;
            }
            0b0100 => {
                let len = ml[i + 4..i + 12].load::<u8>();
                commands.push(Ml32bit::SkipAheadu8(len));
                i += 8;
            }
            0b0101 => {
                let len = ml[i + 4..i + 20].load::<u16>();
                commands.push(Ml32bit::SkipAheadu16(len));
                i += 16;
            }
            0b0010 => {
                let len = ml[i + 4..i + 24].load::<u32>();
                commands.push(Ml32bit::SkipAheadu20(len));
                i += 20;
            }
            0b0011 => {
                let len = ml[i + 4..i + 28].load::<u32>();
                commands.push(Ml32bit::SkipAheadu24(len));
                i += 24;
            }
            0b1001 => {
                let len = ml[i + 4..i + 8].load::<u8>();
                commands.push(Ml32bit::Masku4(len));
                i += 8;
            }
            0b0110 => {
                let len = ml[i + 4..i + 12].load::<u8>();
                commands.push(Ml32bit::Masku8(len));
                i += 8;
            }
            0b0111 => {
                let len = ml[i + 4..i + 20].load::<u16>();
                commands.push(Ml32bit::Masku16(len));
                i += 16;
            }
            0b1100 => {
                let len = ml[i + 4..i + 24].load::<u32>();
                commands.push(Ml32bit::Masku20(len));
                i += 20;
            }
            0b1101 => {
                let len = ml[i + 4..i + 28].load::<u32>();
                commands.push(Ml32bit::Masku24(len));
                i += 24;
            }
            0b1111 => {
                commands.push(Ml32bit::Stop);
                break;
            }
            _ => panic!("Invalid command"),
        };
        i += 4;
    }
    commands
}

/// Must pass in a sequence that contains some masking (lower-case letters)
/// Handles the masking language. If all uppercase, should be handled before this part...
pub fn get_masking_ranges_previous(seq: &[u8]) -> Vec<(usize, usize)> {
    let mut i = 0;
    let mut start = 0;
    let mut end;
    let mut is_masking = false;

    let mut ranges = Vec::new();

    while i < seq.len() {
        let c = seq[i];
        if c.is_ascii_lowercase() {
            if !is_masking {
                start = i;
                is_masking = true;
            }
        } else if is_masking {
            end = i;
            ranges.push((start, end));
            is_masking = false;
        }
        i += 1;
    }

    if is_masking {
        end = i;
        ranges.push((start, end));
    }

    ranges
}

/// Must pass in a sequence that contains some masking (lower-case letters)
/// Handles the masking language. If all uppercase, should be handled before this part...
pub fn get_masking_ranges(seq: &[u8]) -> Vec<(usize, usize)> {
    assert!(!seq.is_empty());
    let mut ranges = Vec::new();
    let mut is_masked = seq[0].is_ascii_lowercase();
    let mut pos = 0;

    while pos < seq.len() {
        let idx =
        if is_masked {
            seq[pos..].iter().position(|x| x.is_ascii_uppercase())
        } else {
            seq[pos..].iter().position(|x| x.is_ascii_lowercase())
        };
        
        if let Some(x) = idx {
            if is_masked {
                ranges.push((pos, pos.saturating_add(x)));
            }
            is_masked = !is_masked;
            pos = pos.saturating_add(x);
        } else {
            break
        }
    }
    
    if is_masked {
        ranges.push((pos, seq.len()));
    }

    ranges
}

pub fn get_masking_ranges2(seq: &[u8]) -> Vec<(usize, usize)> {
    assert!(!seq.is_empty());
    let mut ranges = Vec::new();
    let mut is_masked = seq[0].is_ascii_lowercase();
    let mut pos = 0;

    let len = seq.len();

    while pos < len {
        let idx =
        if is_masked {
            seq[pos..].iter().position(|x| x.is_ascii_uppercase())
        } else {
            seq[pos..].iter().position(|x| x.is_ascii_lowercase())
        };
        
        match idx {
            None => break,
            Some(x) => {
                if is_masked {
                    ranges.push((pos, pos+x));
                }
                is_masked = !is_masked;
                pos += x;
            }
        }
    }
    
    if is_masked {
        ranges.push((pos, seq.len()));
    }

    ranges
    
}


pub fn pad_commands_to_u32(commands: &[Ml32bit]) -> Vec<Ml32bit> {
    let mut padded_commands = Vec::new();
    let mut bitsize: usize = 0;
    for command in commands {
        // This feels gross....
        if bitsize + command.bitsize() == 32 {
            padded_commands.push(*command);
            bitsize = 0;
        } else if bitsize + command.bitsize() > 32 {
            padded_commands.push(Ml32bit::Pass);
            bitsize = 0;
            bitsize = bitsize.saturating_add(command.bitsize());
            padded_commands.push(*command);
        } else {
            bitsize = bitsize.saturating_add(command.bitsize());
            padded_commands.push(*command);
        }
    }

    // Final u32
    padded_commands
}

pub fn convert_ranges_to_ml32bit(ranges: &[(usize, usize)]) -> Vec<Ml32bit> {
    // let mut ml = Vec::new();

    let mut commands: Vec<Ml32bit> = Vec::new();
    let mut cur_pos = 0;
    for (start, end) in ranges {
        let mut skip_len = start - cur_pos;
        let mut mask_len = end - start;

        while skip_len > 0 {
            if skip_len <= 15 {
                commands.push(Ml32bit::SkipAheadu4(skip_len as u8));
                break;
            } else if skip_len <= 255 {
                commands.push(Ml32bit::SkipAheadu8(skip_len as u8));
                break;
            } else if skip_len <= 65535 {
                commands.push(Ml32bit::SkipAheadu16(skip_len as u16));
                break;
            } else if skip_len <= 1048575 {
                commands.push(Ml32bit::SkipAheadu20(skip_len as u32));
                break;
            } else if skip_len <= 16777215 {
                commands.push(Ml32bit::SkipAheadu24(skip_len as u32));
                break;
            } else {
                commands.push(Ml32bit::SkipAheadu24(16777215));
                skip_len = skip_len.saturating_sub(16777215);
            }
        }

        while mask_len > 0 {
            if mask_len <= 15 {
                commands.push(Ml32bit::Masku4(mask_len as u8));
                break;
            } else if mask_len <= 255 {
                commands.push(Ml32bit::Masku8(mask_len as u8));
                break;
            } else if mask_len <= 65535 {
                commands.push(Ml32bit::Masku16(mask_len as u16));
                break;
            } else if mask_len <= 1048575 {
                commands.push(Ml32bit::Masku20(mask_len as u32));
                break;
            } else if mask_len <= 16777215 {
                commands.push(Ml32bit::Masku24(mask_len as u32));
                break;
            } else {
                commands.push(Ml32bit::Masku24(16777215));
                mask_len = mask_len.saturating_sub(16777215);
            }
        }
        cur_pos = *end;
    }

    commands.push(Ml32bit::Stop);
    commands
}

pub fn convert_commands_to_u32(commands: &[Ml32bit]) -> Vec<u32> {
    let mut u32s = Vec::new();
    let mut i = 0;
    let mut cur_u32 = 0u32;
    let mut cur_u32_bits = cur_u32.view_bits_mut::<Lsb0>();

    for command in commands {
        match command {
            Ml32bit::Pass => {
                cur_u32_bits[i..i + 4].store(0b0000);
                u32s.push(cur_u32);
                cur_u32 = 0u32;
                cur_u32_bits = cur_u32.view_bits_mut::<Lsb0>();
                i = 0;
            }
            Ml32bit::SkipAheadu4(len) => {
                cur_u32_bits[i..i + 4].store(0b1000);
                cur_u32_bits[i + 4..i + 8].store(*len);
                i += 8;
            }
            Ml32bit::SkipAheadu8(len) => {
                cur_u32_bits[i..i + 4].store(0b0100);
                cur_u32_bits[i + 4..i + 12].store(*len as u8);
                i += 12;
            }
            Ml32bit::SkipAheadu16(len) => {
                cur_u32_bits[i..i + 4].store(0b0101);
                cur_u32_bits[i + 4..i + 20].store(*len as u16);
                i += 20;
            }
            Ml32bit::SkipAheadu20(len) => {
                cur_u32_bits[i..i + 4].store(0b0110);
                cur_u32_bits[i + 4..i + 24].store(*len as u32);
                i += 24;
            }
            Ml32bit::SkipAheadu24(len) => {
                cur_u32_bits[i..i + 4].store(0b0111);
                cur_u32_bits[i + 4..i + 28].store(*len as u32);
                i += 28;
            }
            Ml32bit::Masku4(len) => {
                cur_u32_bits[i..i + 4].store(0b1001);
                cur_u32_bits[i + 4..i + 8].store(*len);
                i += 8;
            }
            Ml32bit::Masku8(len) => {
                cur_u32_bits[i..i + 4].store(0b0110);
                cur_u32_bits[i + 4..i + 12].store(*len as u8);
                i += 12;
            }
            Ml32bit::Masku16(len) => {
                cur_u32_bits[i..i + 4].store(0b0111);
                cur_u32_bits[i + 4..i + 20].store(*len as u16);
                i += 20;
            }
            Ml32bit::Masku20(len) => {
                cur_u32_bits[i..i + 4].store(0b1000);
                cur_u32_bits[i + 4..i + 24].store(*len as u32);
                i += 24;
            }
            Ml32bit::Masku24(len) => {
                cur_u32_bits[i..i + 4].store(0b1001);
                cur_u32_bits[i + 4..i + 28].store(*len as u32);
                i += 28;
            }
            Ml32bit::Stop => {
                cur_u32_bits[i..i + 4].store(0b1111);
                u32s.push(cur_u32);
                cur_u32 = 0u32;
                cur_u32_bits = cur_u32.view_bits_mut::<Lsb0>();
                i = 0;
            }
        }

        if i >= 32 {
            u32s.push(cur_u32);
            cur_u32 = 0u32;
            cur_u32_bits = cur_u32.view_bits_mut::<Lsb0>();
            i = 0;
        }
    }

    if i > 0 {
        u32s.push(cur_u32);
    }

    u32s
}

pub fn convert_u32_to_commands(u32s: &[u32]) -> Vec<Ml32bit> {
    let mut commands = Vec::new();
    let mut i;

    for cur_u32 in u32s {
        let cur_u32_bits = cur_u32.view_bits::<Lsb0>();
        i = 0;
        while i < 32 {
            let command = match cur_u32_bits[i..i + 4].load::<u8>() {
                0b0000 => {
                    i = 32;
                    continue;
                } // We don't need the pass commands
                0b1000 => {
                    let len = cur_u32_bits[i + 4..i + 8].load::<u8>();
                    i += 8;
                    Ml32bit::SkipAheadu4(len)
                }
                0b0100 => {
                    let len = cur_u32_bits[i + 4..i + 12].load::<u8>();
                    i += 12;
                    Ml32bit::SkipAheadu8(len)
                }
                0b0101 => {
                    let len = cur_u32_bits[i + 4..i + 20].load::<u16>();
                    i += 20;
                    Ml32bit::SkipAheadu16(len)
                }
                0b0010 => {
                    let len = cur_u32_bits[i + 4..i + 24].load::<u32>();
                    i += 24;
                    Ml32bit::SkipAheadu20(len)
                }
                0b0011 => {
                    let len = cur_u32_bits[i + 4..i + 28].load::<u32>();
                    i += 28;
                    Ml32bit::SkipAheadu24(len)
                }
                0b1001 => {
                    let len = cur_u32_bits[i + 4..i + 8].load::<u8>();
                    i += 8;
                    Ml32bit::Masku4(len)
                }
                0b0110 => {
                    let len = cur_u32_bits[i + 4..i + 12].load::<u8>();
                    i += 12;
                    Ml32bit::Masku8(len)
                }
                0b0111 => {
                    let len = cur_u32_bits[i + 4..i + 20].load::<u16>();
                    i += 20;
                    Ml32bit::Masku16(len)
                }
                0b1100 => {
                    let len = cur_u32_bits[i + 4..i + 24].load::<u32>();
                    i += 24;
                    Ml32bit::Masku20(len)
                }
                0b1101 => {
                    let len = cur_u32_bits[i + 4..i + 28].load::<u32>();
                    i += 28;
                    Ml32bit::Masku24(len)
                }
                0b1111 => {
                    // We don't need the stop command AND it means the end of the commands...
                    break;
                }
                _ => panic!("Invalid command"),
            };
            commands.push(command);
        }
    }
    commands
}

pub fn mask_sequence(commands: &[Ml32bit], seq: &mut [u8]) {
    let mut i = 0;

    for command in commands {
        match command {
            Ml32bit::Pass => panic!("Should not receive a pass command here"),
            Ml32bit::SkipAheadu4(len) => {
                i += *len as usize;
            }
            Ml32bit::SkipAheadu8(len) => {
                i += *len as usize;
            }
            Ml32bit::SkipAheadu16(len) => {
                i += *len as usize;
            }
            Ml32bit::SkipAheadu20(len) => {
                i += *len as usize;
            }
            Ml32bit::SkipAheadu24(len) => {
                i += *len as usize;
            }
            Ml32bit::Masku4(len) => {
                seq[i..i + *len as usize].make_ascii_lowercase();
                i += *len as usize;
            }
            Ml32bit::Masku8(len) => {
                seq[i..i + *len as usize].make_ascii_lowercase();
                i += *len as usize;
            }
            Ml32bit::Masku16(len) => {
                seq[i..i + *len as usize].make_ascii_lowercase();
                i += *len as usize;
            }
            Ml32bit::Masku20(len) => {
                seq[i..i + *len as usize].make_ascii_lowercase();
                i += *len as usize;
            }
            Ml32bit::Masku24(len) => {
                seq[i..i + *len as usize].make_ascii_lowercase();
                i += *len as usize;
            }
            Ml32bit::Stop => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::*;

    #[test]
    fn test_parse_masking() {
        let test_seqs = vec![
            "ATCGGGGCAACTACTACGATCAcccccccccaccatgcacatcatctacAAAActcgacaAcatcgacgactacgaa",
            "aaaaaaaaaaaaTACTACGATCAcccccccccaccatgcacatcatctacAAAActcgacaAcatcgacgactACGA",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",           
            "ccaaattgaaagtgagtgtctaatgtattaattagtgaaataatatcttgatatttctttaagggagaattctgataaaa
            gcgtaccgtccccggcTCCTGTGAACTCCCTCCCCCCTTTTACCGCTTGATTGTACTAGAGTCGGTTTGTAGTCATCTTC
            CAGTTCTAAGCGTGTCACTCATGTGAAGTTAGTAGTTTCGTTTGTAGATCTGGCTAACTAAGGtgattaagttttatata
            attttttttaagttttgctaaaaatcattttagaagatattttttaaaaatttatgttcttttatgtggtcctttctcaa
            aatatattgtactgtatatttattataataaagtaccgtatttagttattaaaaatcagCTTGATCGTGTAATAAaaaca
            caggaaaaaaataaaatttatacaacaaattgtgaaatattaatattacaatgataaaaaataaagttatgaattaaaaa
            tagccTAGGGCCTAGGCTATTAGAAATTGACTTACACATTTGATAACGTGACTCACTATAATGatagattttaatgtatt
            ataaaataatttggcaaCCAAAGTACGATTTTATCCATGTTTATGCATACCGCGTCTGACTATAAATCGTTGCATGTGTG
            GAGTACGTTTTCTTTGATCAGGTCTGTCAAACTTCATCGACAATTTAGCTTTAGACCGGTCTTCAGATAAATTGTTGCAT
            TATGTGGGGGCGTAAGAGAACAGTAAGAACTCAATAATctaatagcctaatttttatttttggacatTTCAAAGCACTCT
            AAGAGAAAAGTGACACTCAATTAGCTGTAGGATATATGAGTACTGATATAGTTTTTTATGACCCTGttttagactttgaa
            attttctttcattgtcatTATTTACGGGATTTGTACAAACTGTGCTGGTGCAAGGCTATTTCAGTAACGAGTGTGTCTGA
            TTTATGGCCCAGGAAATTTAAGACTATCTCAATTCTGAATTGAATGGGTTAACAAAGGCAGCAGTTATAAcctgaaataa
            acaattttaaaataacacaaatgctgtccaaaatattttttaattttataaaatgttatttagccTAATACCAGTAGTAG
            GTGACCCTAGTACAGCCATCAGTAGCCTATGATTCAGCAATATGATAAGATACAACAAATGTTTAATTGTATTCAAACTA
            AATATGTAAAaccttgaacattttttttgtcatatacaataaatactattAACAAAATACATTCCTCCGTGCCCCCTCCT
            CTCCCCTGGAATTAATAATTGctgattttgatttaaattaattttctgatttaagtcatatctttaattataattttggg
            tCAAATTATCTCACAAACCATgcataatcatattaaaaaaaatgcgctgtatttgtacttttaaaaaaaaatcactactc
            GTGAGAATCATGAGCAAAATATTCTAAAGTGGAAACGGCACTAAGGTGAACTAAGCAACTTAGTGCAAAActaaatgtta
            gaaaaaatatCCTACACTGCATAAACTATTTTGcaccataaaaaaaagttatgtgtgGGTCTAAAATAATTTGCTGAGCA
            ATTAATGATTTCTAAATGATGCTAAAGTGAACCATTGTAatgttatatgaaaaataaatacacaattaagATCAACACAG
            TGAAATAACATTGATTGGGTGATTTCAAATGGGGTCTATctgaataatgttttatttaacagtaatttttatttctatca
            atttttagtaatatctacaaatattttgttttaggcTGCCAGAAGATCGGCGGTGCAAGGTCAGAGGTGAGATGTTAGGT
            GGTTCCACCAACTGCACGGAAGAGCTGCCCTCTGTCATTCAAAATTTGACAGGTACAAACAGactatattaaataagaaa
            aacaaactttttaaaggCTTGACCATTAGTGAATAGGTTATATGCTTATTATTTCCATTTAGCTTTTTGAGACTAGTATG
            ATTAGACAAATCTGCTTAGttcattttcatataatattgaGGAACAAAATTTGTGAGATTTTGCTAAAATAACTTGCTTT
            GCTTGTTTATAGAGGCacagtaaatcttttttattattattataattttagattttttaatttttaaataagtgataGCA
            TAtgctgtattattaaaaatttaagaactttaaagtatttacagtagcctatattatgtaaTAGGCTAGCCTACTTTATT
            GTTCGGccaattctttttcttattcatCTTATCATTATTagcagattattatttattactagtttaaaagcacgtcaaaa
            atgacggacattaatttcttcctccttaggttgctatacttaaatgccggtcaaaaatttaaaagcccgtcaaaaataac
            agacagcgacatctatggacagaaaatagagagaaacaTCTTCGGGCaacatcggctcgccgaagatggccgagcagtat
            gacagacatcatgaccaaactttctctttattatagttagattagatagaagattagaagactagtttaaaagcccatca
            aaaatgaNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNgttatattgttaaacatctttacttaactctcccagaaaaaaaagttgttattcctctttata
            ccaaattgaaagtgagtgtctaatgtattaattagtgaaataatatcttgatatttctttaagggagaattctgataaaa
            gcgtaccgtccccggcTCCTGTGAACTCCCTCCCCCCTTTTACCGCTTGATTGTACTAGAGTCGGTTTGTAGTCATCTTC
            CAGTTCTAAGCGTGTCACTCATGTGAAGTTAGTAGTTTCGTTTGTAGATCTGGCTAACTAAGGtgattaagttttatata
            attttttttaagttttgctaaaaatcattttagaagatattttttaaaaatttatgttcttttatgtggtcctttctcaa
            aatatattgtactgtatatttattataataaagtaccgtatttagttattaaaaatcagCTTGATCGTGTAATAAaaaca
            caggaaaaaaataaaatttatacaacaaattgtgaaatattaatattacaatgataaaaaataaagttatgaattaaaaa
            tagccTAGGGCCTAGGCTATTAGAAATTGACTTACACATTTGATAACGTGACTCACTATAATGatagattttaatgtatt
            ataaaataatttggcaaCCAAAGTACGATTTTATCCATGTTTATGCATACCGCGTCTGACTATAAATCGTTGCATGTGTG
            GAGTACGTTTTCTTTGATCAGGTCTGTCAAACTTCATCGACAATTTAGCTTTAGACCGGTCTTCAGATAAATTGTTGCAT
            TATGTGGGGGCGTAAGAGAACAGTAAGAACTCAATAATctaatagcctaatttttatttttggacatTTCAAAGCACTCT
            AAGAGAAAAGTGACACTCAATTAGCTGTAGGATATATGAGTACTGATATAGTTTTTTATGACCCTGttttagactttgaa
            attttctttcattgtcatTATTTACGGGATTTGTACAAACTGTGCTGGTGCAAGGCTATTTCAGTAACGAGTGTGTCTGA
            TTTATGGCCCAGGAAATTTAAGACTATCTCAATTCTGAATTGAATGGGTTAACAAAGGCAGCAGTTATAAcctgaaataa
            acaattttaaaataacacaaatgctgtccaaaatattttttaattttataaaatgttatttagccTAATACCAGTAGTAG
            GTGACCCTAGTACAGCCATCAGTAGCCTATGATTCAGCAATATGATAAGATACAACAAATGTTTAATTGTATTCAAACTA
            AATATGTAAAaccttgaacattttttttgtcatatacaataaatactattAACAAAATACATTCCTCCGTGCCCCCTCCT
            CTCCCCTGGAATTAATAATTGctgattttgatttaaattaattttctgatttaagtcatatctttaattataattttggg
            tCAAATTATCTCACAAACCATgcataatcatattaaaaaaaatgcgctgtatttgtacttttaaaaaaaaatcactactc
            GTGAGAATCATGAGCAAAATATTCTAAAGTGGAAACGGCACTAAGGTGAACTAAGCAACTTAGTGCAAAActaaatgtta
            gaaaaaatatCCTACACTGCATAAACTATTTTGcaccataaaaaaaagttatgtgtgGGTCTAAAATAATTTGCTGAGCA
            ATTAATGATTTCTAAATGATGCTAAAGTGAACCATTGTAatgttatatgaaaaataaatacacaattaagATCAACACAG
            TGAAATAACATTGATTGGGTGATTTCAAATGGGGTCTATctgaataatgttttatttaacagtaatttttatttctatca
            atttttagtaatatctacaaatattttgttttaggcTGCCAGAAGATCGGCGGTGCAAGGTCAGAGGTGAGATGTTAGGT
            GGTTCCACCAACTGCACGGAAGAGCTGCCCTCTGTCATTCAAAATTTGACAGGTACAAACAGactatattaaataagaaa
            aacaaactttttaaaggCTTGACCATTAGTGAATAGGTTATATGCTTATTATTTCCATTTAGCTTTTTGAGACTAGTATG
            ATTAGACAAATCTGCTTAGttcattttcatataatattgaGGAACAAAATTTGTGAGATTTTGCTAAAATAACTTGCTTT
            GCTTGTTTATAGAGGCacagtaaatcttttttattattattataattttagattttttaatttttaaataagtgataGCA
            TAtgctgtattattaaaaatttaagaactttaaagtatttacagtagcctatattatgtaaTAGGCTAGCCTACTTTATT
            GTTCGGccaattctttttcttattcatCTTATCATTATTagcagattattatttattactagtttaaaagcacgtcaaaa
            atgacggacattaatttcttcctccttaggttgctatacttaaatgccggtcaaaaatttaaaagcccgtcaaaaataac
            agacagcgacatctatggacagaaaatagagagaaacaTCTTCGGGCaacatcggctcgccgaagatggccgagcagtat
            gacagacatcatgaccaaactttctctttattatagttagattagatagaagattagaagactagtttaaaagcccatca
            aaaatgaNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNgttatattgttaaacatctttacttaactctcccagaaaaaaaagttgttattcctctttataccaaattgaaagtgagtgtctaatgtattaattagtgaaataatatcttgatatttctttaagggagaattctgataaaa
            gcgtaccgtccccggcTCCTGTGAACTCCCTCCCCCCTTTTACCGCTTGATTGTACTAGAGTCGGTTTGTAGTCATCTTC
            CAGTTCTAAGCGTGTCACTCATGTGAAGTTAGTAGTTTCGTTTGTAGATCTGGCTAACTAAGGtgattaagttttatata
            attttttttaagttttgctaaaaatcattttagaagatattttttaaaaatttatgttcttttatgtggtcctttctcaa
            aatatattgtactgtatatttattataataaagtaccgtatttagttattaaaaatcagCTTGATCGTGTAATAAaaaca
            caggaaaaaaataaaatttatacaacaaattgtgaaatattaatattacaatgataaaaaataaagttatgaattaaaaa
            tagccTAGGGCCTAGGCTATTAGAAATTGACTTACACATTTGATAACGTGACTCACTATAATGatagattttaatgtatt
            ataaaataatttggcaaCCAAAGTACGATTTTATCCATGTTTATGCATACCGCGTCTGACTATAAATCGTTGCATGTGTG
            GAGTACGTTTTCTTTGATCAGGTCTGTCAAACTTCATCGACAATTTAGCTTTAGACCGGTCTTCAGATAAATTGTTGCAT
            TATGTGGGGGCGTAAGAGAACAGTAAGAACTCAATAATctaatagcctaatttttatttttggacatTTCAAAGCACTCT
            AAGAGAAAAGTGACACTCAATTAGCTGTAGGATATATGAGTACTGATATAGTTTTTTATGACCCTGttttagactttgaa
            attttctttcattgtcatTATTTACGGGATTTGTACAAACTGTGCTGGTGCAAGGCTATTTCAGTAACGAGTGTGTCTGA
            TTTATGGCCCAGGAAATTTAAGACTATCTCAATTCTGAATTGAATGGGTTAACAAAGGCAGCAGTTATAAcctgaaataa
            acaattttaaaataacacaaatgctgtccaaaatattttttaattttataaaatgttatttagccTAATACCAGTAGTAG
            GTGACCCTAGTACAGCCATCAGTAGCCTATGATTCAGCAATATGATAAGATACAACAAATGTTTAATTGTATTCAAACTA
            AATATGTAAAaccttgaacattttttttgtcatatacaataaatactattAACAAAATACATTCCTCCGTGCCCCCTCCT
            CTCCCCTGGAATTAATAATTGctgattttgatttaaattaattttctgatttaagtcatatctttaattataattttggg
            tCAAATTATCTCACAAACCATgcataatcatattaaaaaaaatgcgctgtatttgtacttttaaaaaaaaatcactactc
            GTGAGAATCATGAGCAAAATATTCTAAAGTGGAAACGGCACTAAGGTGAACTAAGCAACTTAGTGCAAAActaaatgtta
            gaaaaaatatCCTACACTGCATAAACTATTTTGcaccataaaaaaaagttatgtgtgGGTCTAAAATAATTTGCTGAGCA
            ATTAATGATTTCTAAATGATGCTAAAGTGAACCATTGTAatgttatatgaaaaataaatacacaattaagATCAACACAG
            TGAAATAACATTGATTGGGTGATTTCAAATGGGGTCTATctgaataatgttttatttaacagtaatttttatttctatca
            atttttagtaatatctacaaatattttgttttaggcTGCCAGAAGATCGGCGGTGCAAGGTCAGAGGTGAGATGTTAGGT
            GGTTCCACCAACTGCACGGAAGAGCTGCCCTCTGTCATTCAAAATTTGACAGGTACAAACAGactatattaaataagaaa
            aacaaactttttaaaggCTTGACCATTAGTGAATAGGTTATATGCTTATTATTTCCATTTAGCTTTTTGAGACTAGTATG
            ATTAGACAAATCTGCTTAGttcattttcatataatattgaGGAACAAAATTTGTGAGATTTTGCTAAAATAACTTGCTTT
            GCTTGTTTATAGAGGCacagtaaatcttttttattattattataattttagattttttaatttttaaataagtgataGCA
            TAtgctgtattattaaaaatttaagaactttaaagtatttacagtagcctatattatgtaaTAGGCTAGCCTACTTTATT
            GTTCGGccaattctttttcttattcatCTTATCATTATTagcagattattatttattactagtttaaaagcacgtcaaaa
            atgacggacattaatttcttcctccttaggttgctatacttaaatgccggtcaaaaatttaaaagcccgtcaaaaataac
            agacagcgacatctatggacagaaaatagagagaaacaTCTTCGGGCaacatcggctcgccgaagatggccgagcagtat
            gacagacatcatgaccaaactttctctttattatagttagattagatagaagattagaagactagtttaaaagcccatca
            aaaatgaNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNgttatattgttaaacatctttacttaactctcccagaaaaaaaagttgttattcctctttataccaaattgaaagtgagtgtctaatgtattaattagtgaaataatatcttgatatttctttaagggagaattctgataaaa
            gcgtaccgtccccggcTCCTGTGAACTCCCTCCCCCCTTTTACCGCTTGATTGTACTAGAGTCGGTTTGTAGTCATCTTC
            CAGTTCTAAGCGTGTCACTCATGTGAAGTTAGTAGTTTCGTTTGTAGATCTGGCTAACTAAGGtgattaagttttatata
            attttttttaagttttgctaaaaatcattttagaagatattttttaaaaatttatgttcttttatgtggtcctttctcaa
            aatatattgtactgtatatttattataataaagtaccgtatttagttattaaaaatcagCTTGATCGTGTAATAAaaaca
            caggaaaaaaataaaatttatacaacaaattgtgaaatattaatattacaatgataaaaaataaagttatgaattaaaaa
            tagccTAGGGCCTAGGCTATTAGAAATTGACTTACACATTTGATAACGTGACTCACTATAATGatagattttaatgtatt
            ataaaataatttggcaaCCAAAGTACGATTTTATCCATGTTTATGCATACCGCGTCTGACTATAAATCGTTGCATGTGTG
            GAGTACGTTTTCTTTGATCAGGTCTGTCAAACTTCATCGACAATTTAGCTTTAGACCGGTCTTCAGATAAATTGTTGCAT
            TATGTGGGGGCGTAAGAGAACAGTAAGAACTCAATAATctaatagcctaatttttatttttggacatTTCAAAGCACTCT
            AAGAGAAAAGTGACACTCAATTAGCTGTAGGATATATGAGTACTGATATAGTTTTTTATGACCCTGttttagactttgaa
            attttctttcattgtcatTATTTACGGGATTTGTACAAACTGTGCTGGTGCAAGGCTATTTCAGTAACGAGTGTGTCTGA
            TTTATGGCCCAGGAAATTTAAGACTATCTCAATTCTGAATTGAATGGGTTAACAAAGGCAGCAGTTATAAcctgaaataa
            acaattttaaaataacacaaatgctgtccaaaatattttttaattttataaaatgttatttagccTAATACCAGTAGTAG
            GTGACCCTAGTACAGCCATCAGTAGCCTATGATTCAGCAATATGATAAGATACAACAAATGTTTAATTGTATTCAAACTA
            AATATGTAAAaccttgaacattttttttgtcatatacaataaatactattAACAAAATACATTCCTCCGTGCCCCCTCCT
            CTCCCCTGGAATTAATAATTGctgattttgatttaaattaattttctgatttaagtcatatctttaattataattttggg
            tCAAATTATCTCACAAACCATgcataatcatattaaaaaaaatgcgctgtatttgtacttttaaaaaaaaatcactactc
            GTGAGAATCATGAGCAAAATATTCTAAAGTGGAAACGGCACTAAGGTGAACTAAGCAACTTAGTGCAAAActaaatgtta
            gaaaaaatatCCTACACTGCATAAACTATTTTGcaccataaaaaaaagttatgtgtgGGTCTAAAATAATTTGCTGAGCA
            ATTAATGATTTCTAAATGATGCTAAAGTGAACCATTGTAatgttatatgaaaaataaatacacaattaagATCAACACAG
            TGAAATAACATTGATTGGGTGATTTCAAATGGGGTCTATctgaataatgttttatttaacagtaatttttatttctatca
            atttttagtaatatctacaaatattttgttttaggcTGCCAGAAGATCGGCGGTGCAAGGTCAGAGGTGAGATGTTAGGT
            GGTTCCACCAACTGCACGGAAGAGCTGCCCTCTGTCATTCAAAATTTGACAGGTACAAACAGactatattaaataagaaa
            aacaaactttttaaaggCTTGACCATTAGTGAATAGGTTATATGCTTATTATTTCCATTTAGCTTTTTGAGACTAGTATG
            ATTAGACAAATCTGCTTAGttcattttcatataatattgaGGAACAAAATTTGTGAGATTTTGCTAAAATAACTTGCTTT
            GCTTGTTTATAGAGGCacagtaaatcttttttattattattataattttagattttttaatttttaaataagtgataGCA
            TAtgctgtattattaaaaatttaagaactttaaagtatttacagtagcctatattatgtaaTAGGCTAGCCTACTTTATT
            GTTCGGccaattctttttcttattcatCTTATCATTATTagcagattattatttattactagtttaaaagcacgtcaaaa
            atgacggacattaatttcttcctccttaggttgctatacttaaatgccggtcaaaaatttaaaagcccgtcaaaaataac
            agacagcgacatctatggacagaaaatagagagaaacaTCTTCGGGCaacatcggctcgccgaagatggccgagcagtat
            gacagacatcatgaccaaactttctctttattatagttagattagatagaagattagaagactagtttaaaagcccatca
            aaaatgaNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNgttatattgttaaacatctttacttaactctcccagaaaaaaaagttgttattcctctttataccaaattgaaagtgagtgtctaatgtattaattagtgaaataatatcttgatatttctttaagggagaattctgataaaa
            gcgtaccgtccccggcTCCTGTGAACTCCCTCCCCCCTTTTACCGCTTGATTGTACTAGAGTCGGTTTGTAGTCATCTTC
            CAGTTCTAAGCGTGTCACTCATGTGAAGTTAGTAGTTTCGTTTGTAGATCTGGCTAACTAAGGtgattaagttttatata
            attttttttaagttttgctaaaaatcattttagaagatattttttaaaaatttatgttcttttatgtggtcctttctcaa
            aatatattgtactgtatatttattataataaagtaccgtatttagttattaaaaatcagCTTGATCGTGTAATAAaaaca
            caggaaaaaaataaaatttatacaacaaattgtgaaatattaatattacaatgataaaaaataaagttatgaattaaaaa
            tagccTAGGGCCTAGGCTATTAGAAATTGACTTACACATTTGATAACGTGACTCACTATAATGatagattttaatgtatt
            ataaaataatttggcaaCCAAAGTACGATTTTATCCATGTTTATGCATACCGCGTCTGACTATAAATCGTTGCATGTGTG
            GAGTACGTTTTCTTTGATCAGGTCTGTCAAACTTCATCGACAATTTAGCTTTAGACCGGTCTTCAGATAAATTGTTGCAT
            TATGTGGGGGCGTAAGAGAACAGTAAGAACTCAATAATctaatagcctaatttttatttttggacatTTCAAAGCACTCT
            AAGAGAAAAGTGACACTCAATTAGCTGTAGGATATATGAGTACTGATATAGTTTTTTATGACCCTGttttagactttgaa
            attttctttcattgtcatTATTTACGGGATTTGTACAAACTGTGCTGGTGCAAGGCTATTTCAGTAACGAGTGTGTCTGA
            TTTATGGCCCAGGAAATTTAAGACTATCTCAATTCTGAATTGAATGGGTTAACAAAGGCAGCAGTTATAAcctgaaataa
            acaattttaaaataacacaaatgctgtccaaaatattttttaattttataaaatgttatttagccTAATACCAGTAGTAG
            GTGACCCTAGTACAGCCATCAGTAGCCTATGATTCAGCAATATGATAAGATACAACAAATGTTTAATTGTATTCAAACTA
            AATATGTAAAaccttgaacattttttttgtcatatacaataaatactattAACAAAATACATTCCTCCGTGCCCCCTCCT
            CTCCCCTGGAATTAATAATTGctgattttgatttaaattaattttctgatttaagtcatatctttaattataattttggg
            tCAAATTATCTCACAAACCATgcataatcatattaaaaaaaatgcgctgtatttgtacttttaaaaaaaaatcactactc
            GTGAGAATCATGAGCAAAATATTCTAAAGTGGAAACGGCACTAAGGTGAACTAAGCAACTTAGTGCAAAActaaatgtta
            gaaaaaatatCCTACACTGCATAAACTATTTTGcaccataaaaaaaagttatgtgtgGGTCTAAAATAATTTGCTGAGCA
            ATTAATGATTTCTAAATGATGCTAAAGTGAACCATTGTAatgttatatgaaaaataaatacacaattaagATCAACACAG
            TGAAATAACATTGATTGGGTGATTTCAAATGGGGTCTATctgaataatgttttatttaacagtaatttttatttctatca
            atttttagtaatatctacaaatattttgttttaggcTGCCAGAAGATCGGCGGTGCAAGGTCAGAGGTGAGATGTTAGGT
            GGTTCCACCAACTGCACGGAAGAGCTGCCCTCTGTCATTCAAAATTTGACAGGTACAAACAGactatattaaataagaaa
            aacaaactttttaaaggCTTGACCATTAGTGAATAGGTTATATGCTTATTATTTCCATTTAGCTTTTTGAGACTAGTATG
            ATTAGACAAATCTGCTTAGttcattttcatataatattgaGGAACAAAATTTGTGAGATTTTGCTAAAATAACTTGCTTT
            GCTTGTTTATAGAGGCacagtaaatcttttttattattattataattttagattttttaatttttaaataagtgataGCA
            TAtgctgtattattaaaaatttaagaactttaaagtatttacagtagcctatattatgtaaTAGGCTAGCCTACTTTATT
            GTTCGGccaattctttttcttattcatCTTATCATTATTagcagattattatttattactagtttaaaagcacgtcaaaa
            atgacggacattaatttcttcctccttaggttgctatacttaaatgccggtcaaaaatttaaaagcccgtcaaaaataac
            agacagcgacatctatggacagaaaatagagagaaacaTCTTCGGGCaacatcggctcgccgaagatggccgagcagtat
            gacagacatcatgaccaaactttctctttattatagttagattagatagaagattagaagactagtttaaaagcccatca
            aaaatgaNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNgttatattgttaaacatctttacttaactctcccagaaaaaaaagttgttattcctctttataccaaattgaaagtgagtgtctaatgtattaattagtgaaataatatcttgatatttctttaagggagaattctgataaaa
            gcgtaccgtccccggcTCCTGTGAACTCCCTCCCCCCTTTTACCGCTTGATTGTACTAGAGTCGGTTTGTAGTCATCTTC
            CAGTTCTAAGCGTGTCACTCATGTGAAGTTAGTAGTTTCGTTTGTAGATCTGGCTAACTAAGGtgattaagttttatata
            attttttttaagttttgctaaaaatcattttagaagatattttttaaaaatttatgttcttttatgtggtcctttctcaa
            aatatattgtactgtatatttattataataaagtaccgtatttagttattaaaaatcagCTTGATCGTGTAATAAaaaca
            caggaaaaaaataaaatttatacaacaaattgtgaaatattaatattacaatgataaaaaataaagttatgaattaaaaa
            tagccTAGGGCCTAGGCTATTAGAAATTGACTTACACATTTGATAACGTGACTCACTATAATGatagattttaatgtatt
            ataaaataatttggcaaCCAAAGTACGATTTTATCCATGTTTATGCATACCGCGTCTGACTATAAATCGTTGCATGTGTG
            GAGTACGTTTTCTTTGATCAGGTCTGTCAAACTTCATCGACAATTTAGCTTTAGACCGGTCTTCAGATAAATTGTTGCAT
            TATGTGGGGGCGTAAGAGAACAGTAAGAACTCAATAATctaatagcctaatttttatttttggacatTTCAAAGCACTCT
            AAGAGAAAAGTGACACTCAATTAGCTGTAGGATATATGAGTACTGATATAGTTTTTTATGACCCTGttttagactttgaa
            attttctttcattgtcatTATTTACGGGATTTGTACAAACTGTGCTGGTGCAAGGCTATTTCAGTAACGAGTGTGTCTGA
            TTTATGGCCCAGGAAATTTAAGACTATCTCAATTCTGAATTGAATGGGTTAACAAAGGCAGCAGTTATAAcctgaaataa
            acaattttaaaataacacaaatgctgtccaaaatattttttaattttataaaatgttatttagccTAATACCAGTAGTAG
            GTGACCCTAGTACAGCCATCAGTAGCCTATGATTCAGCAATATGATAAGATACAACAAATGTTTAATTGTATTCAAACTA
            AATATGTAAAaccttgaacattttttttgtcatatacaataaatactattAACAAAATACATTCCTCCGTGCCCCCTCCT
            CTCCCCTGGAATTAATAATTGctgattttgatttaaattaattttctgatttaagtcatatctttaattataattttggg
            tCAAATTATCTCACAAACCATgcataatcatattaaaaaaaatgcgctgtatttgtacttttaaaaaaaaatcactactc
            GTGAGAATCATGAGCAAAATATTCTAAAGTGGAAACGGCACTAAGGTGAACTAAGCAACTTAGTGCAAAActaaatgtta
            gaaaaaatatCCTACACTGCATAAACTATTTTGcaccataaaaaaaagttatgtgtgGGTCTAAAATAATTTGCTGAGCA
            ATTAATGATTTCTAAATGATGCTAAAGTGAACCATTGTAatgttatatgaaaaataaatacacaattaagATCAACACAG
            TGAAATAACATTGATTGGGTGATTTCAAATGGGGTCTATctgaataatgttttatttaacagtaatttttatttctatca
            atttttagtaatatctacaaatattttgttttaggcTGCCAGAAGATCGGCGGTGCAAGGTCAGAGGTGAGATGTTAGGT
            GGTTCCACCAACTGCACGGAAGAGCTGCCCTCTGTCATTCAAAATTTGACAGGTACAAACAGactatattaaataagaaa
            aacaaactttttaaaggCTTGACCATTAGTGAATAGGTTATATGCTTATTATTTCCATTTAGCTTTTTGAGACTAGTATG
            ATTAGACAAATCTGCTTAGttcattttcatataatattgaGGAACAAAATTTGTGAGATTTTGCTAAAATAACTTGCTTT
            GCTTGTTTATAGAGGCacagtaaatcttttttattattattataattttagattttttaatttttaaataagtgataGCA
            TAtgctgtattattaaaaatttaagaactttaaagtatttacagtagcctatattatgtaaTAGGCTAGCCTACTTTATT
            GTTCGGccaattctttttcttattcatCTTATCATTATTagcagattattatttattactagtttaaaagcacgtcaaaa
            atgacggacattaatttcttcctccttaggttgctatacttaaatgccggtcaaaaatttaaaagcccgtcaaaaataac
            agacagcgacatctatggacagaaaatagagagaaacaTCTTCGGGCaacatcggctcgccgaagatggccgagcagtat
            gacagacatcatgaccaaactttctctttattatagttagattagatagaagattagaagactagtttaaaagcccatca
            aaaatgaNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNgttatattgttaaacatctttacttaactctcccagaaaaaaaagttgttattcctctttataccaaattgaaagtgagtgtctaatgtattaattagtgaaataatatcttgatatttctttaagggagaattctgataaaa
            gcgtaccgtccccggcTCCTGTGAACTCCCTCCCCCCTTTTACCGCTTGATTGTACTAGAGTCGGTTTGTAGTCATCTTC
            CAGTTCTAAGCGTGTCACTCATGTGAAGTTAGTAGTTTCGTTTGTAGATCTGGCTAACTAAGGtgattaagttttatata
            attttttttaagttttgctaaaaatcattttagaagatattttttaaaaatttatgttcttttatgtggtcctttctcaa
            aatatattgtactgtatatttattataataaagtaccgtatttagttattaaaaatcagCTTGATCGTGTAATAAaaaca
            caggaaaaaaataaaatttatacaacaaattgtgaaatattaatattacaatgataaaaaataaagttatgaattaaaaa
            tagccTAGGGCCTAGGCTATTAGAAATTGACTTACACATTTGATAACGTGACTCACTATAATGatagattttaatgtatt
            ataaaataatttggcaaCCAAAGTACGATTTTATCCATGTTTATGCATACCGCGTCTGACTATAAATCGTTGCATGTGTG
            GAGTACGTTTTCTTTGATCAGGTCTGTCAAACTTCATCGACAATTTAGCTTTAGACCGGTCTTCAGATAAATTGTTGCAT
            TATGTGGGGGCGTAAGAGAACAGTAAGAACTCAATAATctaatagcctaatttttatttttggacatTTCAAAGCACTCT
            AAGAGAAAAGTGACACTCAATTAGCTGTAGGATATATGAGTACTGATATAGTTTTTTATGACCCTGttttagactttgaa
            attttctttcattgtcatTATTTACGGGATTTGTACAAACTGTGCTGGTGCAAGGCTATTTCAGTAACGAGTGTGTCTGA
            TTTATGGCCCAGGAAATTTAAGACTATCTCAATTCTGAATTGAATGGGTTAACAAAGGCAGCAGTTATAAcctgaaataa
            acaattttaaaataacacaaatgctgtccaaaatattttttaattttataaaatgttatttagccTAATACCAGTAGTAG
            GTGACCCTAGTACAGCCATCAGTAGCCTATGATTCAGCAATATGATAAGATACAACAAATGTTTAATTGTATTCAAACTA
            AATATGTAAAaccttgaacattttttttgtcatatacaataaatactattAACAAAATACATTCCTCCGTGCCCCCTCCT
            CTCCCCTGGAATTAATAATTGctgattttgatttaaattaattttctgatttaagtcatatctttaattataattttggg
            tCAAATTATCTCACAAACCATgcataatcatattaaaaaaaatgcgctgtatttgtacttttaaaaaaaaatcactactc
            GTGAGAATCATGAGCAAAATATTCTAAAGTGGAAACGGCACTAAGGTGAACTAAGCAACTTAGTGCAAAActaaatgtta
            gaaaaaatatCCTACACTGCATAAACTATTTTGcaccataaaaaaaagttatgtgtgGGTCTAAAATAATTTGCTGAGCA
            ATTAATGATTTCTAAATGATGCTAAAGTGAACCATTGTAatgttatatgaaaaataaatacacaattaagATCAACACAG
            TGAAATAACATTGATTGGGTGATTTCAAATGGGGTCTATctgaataatgttttatttaacagtaatttttatttctatca
            atttttagtaatatctacaaatattttgttttaggcTGCCAGAAGATCGGCGGTGCAAGGTCAGAGGTGAGATGTTAGGT
            GGTTCCACCAACTGCACGGAAGAGCTGCCCTCTGTCATTCAAAATTTGACAGGTACAAACAGactatattaaataagaaa
            aacaaactttttaaaggCTTGACCATTAGTGAATAGGTTATATGCTTATTATTTCCATTTAGCTTTTTGAGACTAGTATG
            ATTAGACAAATCTGCTTAGttcattttcatataatattgaGGAACAAAATTTGTGAGATTTTGCTAAAATAACTTGCTTT
            GCTTGTTTATAGAGGCacagtaaatcttttttattattattataattttagattttttaatttttaaataagtgataGCA
            TAtgctgtattattaaaaatttaagaactttaaagtatttacagtagcctatattatgtaaTAGGCTAGCCTACTTTATT
            GTTCGGccaattctttttcttattcatCTTATCATTATTagcagattattatttattactagtttaaaagcacgtcaaaa
            atgacggacattaatttcttcctccttaggttgctatacttaaatgccggtcaaaaatttaaaagcccgtcaaaaataac
            agacagcgacatctatggacagaaaatagagagaaacaTCTTCGGGCaacatcggctcgccgaagatggccgagcagtat
            gacagacatcatgaccaaactttctctttattatagttagattagatagaagattagaagactagtttaaaagcccatca
            aaaatgaNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNgttatattgttaaacatctttacttaactctcccagaaaaaaaagttgttattcctctttataccaaattgaaagtgagtgtctaatgtattaattagtgaaataatatcttgatatttctttaagggagaattctgataaaa
            gcgtaccgtccccggcTCCTGTGAACTCCCTCCCCCCTTTTACCGCTTGATTGTACTAGAGTCGGTTTGTAGTCATCTTC
            CAGTTCTAAGCGTGTCACTCATGTGAAGTTAGTAGTTTCGTTTGTAGATCTGGCTAACTAAGGtgattaagttttatata
            attttttttaagttttgctaaaaatcattttagaagatattttttaaaaatttatgttcttttatgtggtcctttctcaa
            aatatattgtactgtatatttattataataaagtaccgtatttagttattaaaaatcagCTTGATCGTGTAATAAaaaca
            caggaaaaaaataaaatttatacaacaaattgtgaaatattaatattacaatgataaaaaataaagttatgaattaaaaa
            tagccTAGGGCCTAGGCTATTAGAAATTGACTTACACATTTGATAACGTGACTCACTATAATGatagattttaatgtatt
            ataaaataatttggcaaCCAAAGTACGATTTTATCCATGTTTATGCATACCGCGTCTGACTATAAATCGTTGCATGTGTG
            GAGTACGTTTTCTTTGATCAGGTCTGTCAAACTTCATCGACAATTTAGCTTTAGACCGGTCTTCAGATAAATTGTTGCAT
            TATGTGGGGGCGTAAGAGAACAGTAAGAACTCAATAATctaatagcctaatttttatttttggacatTTCAAAGCACTCT
            AAGAGAAAAGTGACACTCAATTAGCTGTAGGATATATGAGTACTGATATAGTTTTTTATGACCCTGttttagactttgaa
            attttctttcattgtcatTATTTACGGGATTTGTACAAACTGTGCTGGTGCAAGGCTATTTCAGTAACGAGTGTGTCTGA
            TTTATGGCCCAGGAAATTTAAGACTATCTCAATTCTGAATTGAATGGGTTAACAAAGGCAGCAGTTATAAcctgaaataa
            acaattttaaaataacacaaatgctgtccaaaatattttttaattttataaaatgttatttagccTAATACCAGTAGTAG
            GTGACCCTAGTACAGCCATCAGTAGCCTATGATTCAGCAATATGATAAGATACAACAAATGTTTAATTGTATTCAAACTA
            AATATGTAAAaccttgaacattttttttgtcatatacaataaatactattAACAAAATACATTCCTCCGTGCCCCCTCCT
            CTCCCCTGGAATTAATAATTGctgattttgatttaaattaattttctgatttaagtcatatctttaattataattttggg
            tCAAATTATCTCACAAACCATgcataatcatattaaaaaaaatgcgctgtatttgtacttttaaaaaaaaatcactactc
            GTGAGAATCATGAGCAAAATATTCTAAAGTGGAAACGGCACTAAGGTGAACTAAGCAACTTAGTGCAAAActaaatgtta
            gaaaaaatatCCTACACTGCATAAACTATTTTGcaccataaaaaaaagttatgtgtgGGTCTAAAATAATTTGCTGAGCA
            ATTAATGATTTCTAAATGATGCTAAAGTGAACCATTGTAatgttatatgaaaaataaatacacaattaagATCAACACAG
            TGAAATAACATTGATTGGGTGATTTCAAATGGGGTCTATctgaataatgttttatttaacagtaatttttatttctatca
            atttttagtaatatctacaaatattttgttttaggcTGCCAGAAGATCGGCGGTGCAAGGTCAGAGGTGAGATGTTAGGT
            GGTTCCACCAACTGCACGGAAGAGCTGCCCTCTGTCATTCAAAATTTGACAGGTACAAACAGactatattaaataagaaa
            aacaaactttttaaaggCTTGACCATTAGTGAATAGGTTATATGCTTATTATTTCCATTTAGCTTTTTGAGACTAGTATG
            ATTAGACAAATCTGCTTAGttcattttcatataatattgaGGAACAAAATTTGTGAGATTTTGCTAAAATAACTTGCTTT
            GCTTGTTTATAGAGGCacagtaaatcttttttattattattataattttagattttttaatttttaaataagtgataGCA
            TAtgctgtattattaaaaatttaagaactttaaagtatttacagtagcctatattatgtaaTAGGCTAGCCTACTTTATT
            GTTCGGccaattctttttcttattcatCTTATCATTATTagcagattattatttattactagtttaaaagcacgtcaaaa
            atgacggacattaatttcttcctccttaggttgctatacttaaatgccggtcaaaaatttaaaagcccgtcaaaaataac
            agacagcgacatctatggacagaaaatagagagaaacaTCTTCGGGCaacatcggctcgccgaagatggccgagcagtat
            gacagacatcatgaccaaactttctctttattatagttagattagatagaagattagaagactagtttaaaagcccatca
            aaaatgaNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNgttatattgttaaacatctttacttaactctcccagaaaaaaaagttgttattcctctttataccaaattgaaagtgagtgtctaatgtattaattagtgaaataatatcttgatatttctttaagggagaattctgataaaa
            gcgtaccgtccccggcTCCTGTGAACTCCCTCCCCCCTTTTACCGCTTGATTGTACTAGAGTCGGTTTGTAGTCATCTTC
            CAGTTCTAAGCGTGTCACTCATGTGAAGTTAGTAGTTTCGTTTGTAGATCTGGCTAACTAAGGtgattaagttttatata
            attttttttaagttttgctaaaaatcattttagaagatattttttaaaaatttatgttcttttatgtggtcctttctcaa
            aatatattgtactgtatatttattataataaagtaccgtatttagttattaaaaatcagCTTGATCGTGTAATAAaaaca
            caggaaaaaaataaaatttatacaacaaattgtgaaatattaatattacaatgataaaaaataaagttatgaattaaaaa
            tagccTAGGGCCTAGGCTATTAGAAATTGACTTACACATTTGATAACGTGACTCACTATAATGatagattttaatgtatt
            ataaaataatttggcaaCCAAAGTACGATTTTATCCATGTTTATGCATACCGCGTCTGACTATAAATCGTTGCATGTGTG
            GAGTACGTTTTCTTTGATCAGGTCTGTCAAACTTCATCGACAATTTAGCTTTAGACCGGTCTTCAGATAAATTGTTGCAT
            TATGTGGGGGCGTAAGAGAACAGTAAGAACTCAATAATctaatagcctaatttttatttttggacatTTCAAAGCACTCT
            AAGAGAAAAGTGACACTCAATTAGCTGTAGGATATATGAGTACTGATATAGTTTTTTATGACCCTGttttagactttgaa
            attttctttcattgtcatTATTTACGGGATTTGTACAAACTGTGCTGGTGCAAGGCTATTTCAGTAACGAGTGTGTCTGA
            TTTATGGCCCAGGAAATTTAAGACTATCTCAATTCTGAATTGAATGGGTTAACAAAGGCAGCAGTTATAAcctgaaataa
            acaattttaaaataacacaaatgctgtccaaaatattttttaattttataaaatgttatttagccTAATACCAGTAGTAG
            GTGACCCTAGTACAGCCATCAGTAGCCTATGATTCAGCAATATGATAAGATACAACAAATGTTTAATTGTATTCAAACTA
            AATATGTAAAaccttgaacattttttttgtcatatacaataaatactattAACAAAATACATTCCTCCGTGCCCCCTCCT
            CTCCCCTGGAATTAATAATTGctgattttgatttaaattaattttctgatttaagtcatatctttaattataattttggg
            tCAAATTATCTCACAAACCATgcataatcatattaaaaaaaatgcgctgtatttgtacttttaaaaaaaaatcactactc
            GTGAGAATCATGAGCAAAATATTCTAAAGTGGAAACGGCACTAAGGTGAACTAAGCAACTTAGTGCAAAActaaatgtta
            gaaaaaatatCCTACACTGCATAAACTATTTTGcaccataaaaaaaagttatgtgtgGGTCTAAAATAATTTGCTGAGCA
            ATTAATGATTTCTAAATGATGCTAAAGTGAACCATTGTAatgttatatgaaaaataaatacacaattaagATCAACACAG
            TGAAATAACATTGATTGGGTGATTTCAAATGGGGTCTATctgaataatgttttatttaacagtaatttttatttctatca
            atttttagtaatatctacaaatattttgttttaggcTGCCAGAAGATCGGCGGTGCAAGGTCAGAGGTGAGATGTTAGGT
            GGTTCCACCAACTGCACGGAAGAGCTGCCCTCTGTCATTCAAAATTTGACAGGTACAAACAGactatattaaataagaaa
            aacaaactttttaaaggCTTGACCATTAGTGAATAGGTTATATGCTTATTATTTCCATTTAGCTTTTTGAGACTAGTATG
            ATTAGACAAATCTGCTTAGttcattttcatataatattgaGGAACAAAATTTGTGAGATTTTGCTAAAATAACTTGCTTT
            GCTTGTTTATAGAGGCacagtaaatcttttttattattattataattttagattttttaatttttaaataagtgataGCA
            TAtgctgtattattaaaaatttaagaactttaaagtatttacagtagcctatattatgtaaTAGGCTAGCCTACTTTATT
            GTTCGGccaattctttttcttattcatCTTATCATTATTagcagattattatttattactagtttaaaagcacgtcaaaa
            atgacggacattaatttcttcctccttaggttgctatacttaaatgccggtcaaaaatttaaaagcccgtcaaaaataac
            agacagcgacatctatggacagaaaatagagagaaacaTCTTCGGGCaacatcggctcgccgaagatggccgagcagtat
            gacagacatcatgaccaaactttctctttattatagttagattagatagaagattagaagactagtttaaaagcccatca
            aaaatgaNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNgttatattgttaaacatctttacttaactctcccagaaaaaaaagttgttattcctctttataccaaattgaaagtgagtgtctaatgtattaattagtgaaataatatcttgatatttctttaagggagaattctgataaaa
            gcgtaccgtccccggcTCCTGTGAACTCCCTCCCCCCTTTTACCGCTTGATTGTACTAGAGTCGGTTTGTAGTCATCTTC
            CAGTTCTAAGCGTGTCACTCATGTGAAGTTAGTAGTTTCGTTTGTAGATCTGGCTAACTAAGGtgattaagttttatata
            attttttttaagttttgctaaaaatcattttagaagatattttttaaaaatttatgttcttttatgtggtcctttctcaa
            aatatattgtactgtatatttattataataaagtaccgtatttagttattaaaaatcagCTTGATCGTGTAATAAaaaca
            caggaaaaaaataaaatttatacaacaaattgtgaaatattaatattacaatgataaaaaataaagttatgaattaaaaa
            tagccTAGGGCCTAGGCTATTAGAAATTGACTTACACATTTGATAACGTGACTCACTATAATGatagattttaatgtatt
            ataaaataatttggcaaCCAAAGTACGATTTTATCCATGTTTATGCATACCGCGTCTGACTATAAATCGTTGCATGTGTG
            GAGTACGTTTTCTTTGATCAGGTCTGTCAAACTTCATCGACAATTTAGCTTTAGACCGGTCTTCAGATAAATTGTTGCAT
            TATGTGGGGGCGTAAGAGAACAGTAAGAACTCAATAATctaatagcctaatttttatttttggacatTTCAAAGCACTCT
            AAGAGAAAAGTGACACTCAATTAGCTGTAGGATATATGAGTACTGATATAGTTTTTTATGACCCTGttttagactttgaa
            attttctttcattgtcatTATTTACGGGATTTGTACAAACTGTGCTGGTGCAAGGCTATTTCAGTAACGAGTGTGTCTGA
            TTTATGGCCCAGGAAATTTAAGACTATCTCAATTCTGAATTGAATGGGTTAACAAAGGCAGCAGTTATAAcctgaaataa
            acaattttaaaataacacaaatgctgtccaaaatattttttaattttataaaatgttatttagccTAATACCAGTAGTAG
            GTGACCCTAGTACAGCCATCAGTAGCCTATGATTCAGCAATATGATAAGATACAACAAATGTTTAATTGTATTCAAACTA
            AATATGTAAAaccttgaacattttttttgtcatatacaataaatactattAACAAAATACATTCCTCCGTGCCCCCTCCT
            CTCCCCTGGAATTAATAATTGctgattttgatttaaattaattttctgatttaagtcatatctttaattataattttggg
            tCAAATTATCTCACAAACCATgcataatcatattaaaaaaaatgcgctgtatttgtacttttaaaaaaaaatcactactc
            GTGAGAATCATGAGCAAAATATTCTAAAGTGGAAACGGCACTAAGGTGAACTAAGCAACTTAGTGCAAAActaaatgtta
            gaaaaaatatCCTACACTGCATAAACTATTTTGcaccataaaaaaaagttatgtgtgGGTCTAAAATAATTTGCTGAGCA
            ATTAATGATTTCTAAATGATGCTAAAGTGAACCATTGTAatgttatatgaaaaataaatacacaattaagATCAACACAG
            TGAAATAACATTGATTGGGTGATTTCAAATGGGGTCTATctgaataatgttttatttaacagtaatttttatttctatca
            atttttagtaatatctacaaatattttgttttaggcTGCCAGAAGATCGGCGGTGCAAGGTCAGAGGTGAGATGTTAGGT
            GGTTCCACCAACTGCACGGAAGAGCTGCCCTCTGTCATTCAAAATTTGACAGGTACAAACAGactatattaaataagaaa
            aacaaactttttaaaggCTTGACCATTAGTGAATAGGTTATATGCTTATTATTTCCATTTAGCTTTTTGAGACTAGTATG
            ATTAGACAAATCTGCTTAGttcattttcatataatattgaGGAACAAAATTTGTGAGATTTTGCTAAAATAACTTGCTTT
            GCTTGTTTATAGAGGCacagtaaatcttttttattattattataattttagattttttaatttttaaataagtgataGCA
            TAtgctgtattattaaaaatttaagaactttaaagtatttacagtagcctatattatgtaaTAGGCTAGCCTACTTTATT
            GTTCGGccaattctttttcttattcatCTTATCATTATTagcagattattatttattactagtttaaaagcacgtcaaaa
            atgacggacattaatttcttcctccttaggttgctatacttaaatgccggtcaaaaatttaaaagcccgtcaaaaataac
            agacagcgacatctatggacagaaaatagagagaaacaTCTTCGGGCaacatcggctcgccgaagatggccgagcagtat
            gacagacatcatgaccaaactttctctttattatagttagattagatagaagattagaagactagtttaaaagcccatca
            aaaatgaNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNgttatattgttaaacatctttacttaactctcccagaaaaaaaagttgttattcctctttataccaaattgaaagtgagtgtctaatgtattaattagtgaaataatatcttgatatttctttaagggagaattctgataaaa
            gcgtaccgtccccggcTCCTGTGAACTCCCTCCCCCCTTTTACCGCTTGATTGTACTAGAGTCGGTTTGTAGTCATCTTC
            CAGTTCTAAGCGTGTCACTCATGTGAAGTTAGTAGTTTCGTTTGTAGATCTGGCTAACTAAGGtgattaagttttatata
            attttttttaagttttgctaaaaatcattttagaagatattttttaaaaatttatgttcttttatgtggtcctttctcaa
            aatatattgtactgtatatttattataataaagtaccgtatttagttattaaaaatcagCTTGATCGTGTAATAAaaaca
            caggaaaaaaataaaatttatacaacaaattgtgaaatattaatattacaatgataaaaaataaagttatgaattaaaaa
            tagccTAGGGCCTAGGCTATTAGAAATTGACTTACACATTTGATAACGTGACTCACTATAATGatagattttaatgtatt
            ataaaataatttggcaaCCAAAGTACGATTTTATCCATGTTTATGCATACCGCGTCTGACTATAAATCGTTGCATGTGTG
            GAGTACGTTTTCTTTGATCAGGTCTGTCAAACTTCATCGACAATTTAGCTTTAGACCGGTCTTCAGATAAATTGTTGCAT
            TATGTGGGGGCGTAAGAGAACAGTAAGAACTCAATAATctaatagcctaatttttatttttggacatTTCAAAGCACTCT
            AAGAGAAAAGTGACACTCAATTAGCTGTAGGATATATGAGTACTGATATAGTTTTTTATGACCCTGttttagactttgaa
            attttctttcattgtcatTATTTACGGGATTTGTACAAACTGTGCTGGTGCAAGGCTATTTCAGTAACGAGTGTGTCTGA
            TTTATGGCCCAGGAAATTTAAGACTATCTCAATTCTGAATTGAATGGGTTAACAAAGGCAGCAGTTATAAcctgaaataa
            acaattttaaaataacacaaatgctgtccaaaatattttttaattttataaaatgttatttagccTAATACCAGTAGTAG
            GTGACCCTAGTACAGCCATCAGTAGCCTATGATTCAGCAATATGATAAGATACAACAAATGTTTAATTGTATTCAAACTA
            AATATGTAAAaccttgaacattttttttgtcatatacaataaatactattAACAAAATACATTCCTCCGTGCCCCCTCCT
            CTCCCCTGGAATTAATAATTGctgattttgatttaaattaattttctgatttaagtcatatctttaattataattttggg
            tCAAATTATCTCACAAACCATgcataatcatattaaaaaaaatgcgctgtatttgtacttttaaaaaaaaatcactactc
            GTGAGAATCATGAGCAAAATATTCTAAAGTGGAAACGGCACTAAGGTGAACTAAGCAACTTAGTGCAAAActaaatgtta
            gaaaaaatatCCTACACTGCATAAACTATTTTGcaccataaaaaaaagttatgtgtgGGTCTAAAATAATTTGCTGAGCA
            ATTAATGATTTCTAAATGATGCTAAAGTGAACCATTGTAatgttatatgaaaaataaatacacaattaagATCAACACAG
            TGAAATAACATTGATTGGGTGATTTCAAATGGGGTCTATctgaataatgttttatttaacagtaatttttatttctatca
            atttttagtaatatctacaaatattttgttttaggcTGCCAGAAGATCGGCGGTGCAAGGTCAGAGGTGAGATGTTAGGT
            GGTTCCACCAACTGCACGGAAGAGCTGCCCTCTGTCATTCAAAATTTGACAGGTACAAACAGactatattaaataagaaa
            aacaaactttttaaaggCTTGACCATTAGTGAATAGGTTATATGCTTATTATTTCCATTTAGCTTTTTGAGACTAGTATG
            ATTAGACAAATCTGCTTAGttcattttcatataatattgaGGAACAAAATTTGTGAGATTTTGCTAAAATAACTTGCTTT
            GCTTGTTTATAGAGGCacagtaaatcttttttattattattataattttagattttttaatttttaaataagtgataGCA
            TAtgctgtattattaaaaatttaagaactttaaagtatttacagtagcctatattatgtaaTAGGCTAGCCTACTTTATT
            GTTCGGccaattctttttcttattcatCTTATCATTATTagcagattattatttattactagtttaaaagcacgtcaaaa
            atgacggacattaatttcttcctccttaggttgctatacttaaatgccggtcaaaaatttaaaagcccgtcaaaaataac
            agacagcgacatctatggacagaaaatagagagaaacaTCTTCGGGCaacatcggctcgccgaagatggccgagcagtat
            gacagacatcatgaccaaactttctctttattatagttagattagatagaagattagaagactagtttaaaagcccatca
            aaaatgaNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNgttatattgttaaacatctttacttaactctcccagaaaaaaaagttgttattcctctttataccaaattgaaagtgagtgtctaatgtattaattagtgaaataatatcttgatatttctttaagggagaattctgataaaa
            gcgtaccgtccccggcTCCTGTGAACTCCCTCCCCCCTTTTACCGCTTGATTGTACTAGAGTCGGTTTGTAGTCATCTTC
            CAGTTCTAAGCGTGTCACTCATGTGAAGTTAGTAGTTTCGTTTGTAGATCTGGCTAACTAAGGtgattaagttttatata
            attttttttaagttttgctaaaaatcattttagaagatattttttaaaaatttatgttcttttatgtggtcctttctcaa
            aatatattgtactgtatatttattataataaagtaccgtatttagttattaaaaatcagCTTGATCGTGTAATAAaaaca
            caggaaaaaaataaaatttatacaacaaattgtgaaatattaatattacaatgataaaaaataaagttatgaattaaaaa
            tagccTAGGGCCTAGGCTATTAGAAATTGACTTACACATTTGATAACGTGACTCACTATAATGatagattttaatgtatt
            ataaaataatttggcaaCCAAAGTACGATTTTATCCATGTTTATGCATACCGCGTCTGACTATAAATCGTTGCATGTGTG
            GAGTACGTTTTCTTTGATCAGGTCTGTCAAACTTCATCGACAATTTAGCTTTAGACCGGTCTTCAGATAAATTGTTGCAT
            TATGTGGGGGCGTAAGAGAACAGTAAGAACTCAATAATctaatagcctaatttttatttttggacatTTCAAAGCACTCT
            AAGAGAAAAGTGACACTCAATTAGCTGTAGGATATATGAGTACTGATATAGTTTTTTATGACCCTGttttagactttgaa
            attttctttcattgtcatTATTTACGGGATTTGTACAAACTGTGCTGGTGCAAGGCTATTTCAGTAACGAGTGTGTCTGA
            TTTATGGCCCAGGAAATTTAAGACTATCTCAATTCTGAATTGAATGGGTTAACAAAGGCAGCAGTTATAAcctgaaataa
            acaattttaaaataacacaaatgctgtccaaaatattttttaattttataaaatgttatttagccTAATACCAGTAGTAG
            GTGACCCTAGTACAGCCATCAGTAGCCTATGATTCAGCAATATGATAAGATACAACAAATGTTTAATTGTATTCAAACTA
            AATATGTAAAaccttgaacattttttttgtcatatacaataaatactattAACAAAATACATTCCTCCGTGCCCCCTCCT
            CTCCCCTGGAATTAATAATTGctgattttgatttaaattaattttctgatttaagtcatatctttaattataattttggg
            tCAAATTATCTCACAAACCATgcataatcatattaaaaaaaatgcgctgtatttgtacttttaaaaaaaaatcactactc
            GTGAGAATCATGAGCAAAATATTCTAAAGTGGAAACGGCACTAAGGTGAACTAAGCAACTTAGTGCAAAActaaatgtta
            gaaaaaatatCCTACACTGCATAAACTATTTTGcaccataaaaaaaagttatgtgtgGGTCTAAAATAATTTGCTGAGCA
            ATTAATGATTTCTAAATGATGCTAAAGTGAACCATTGTAatgttatatgaaaaataaatacacaattaagATCAACACAG
            TGAAATAACATTGATTGGGTGATTTCAAATGGGGTCTATctgaataatgttttatttaacagtaatttttatttctatca
            atttttagtaatatctacaaatattttgttttaggcTGCCAGAAGATCGGCGGTGCAAGGTCAGAGGTGAGATGTTAGGT
            GGTTCCACCAACTGCACGGAAGAGCTGCCCTCTGTCATTCAAAATTTGACAGGTACAAACAGactatattaaataagaaa
            aacaaactttttaaaggCTTGACCATTAGTGAATAGGTTATATGCTTATTATTTCCATTTAGCTTTTTGAGACTAGTATG
            ATTAGACAAATCTGCTTAGttcattttcatataatattgaGGAACAAAATTTGTGAGATTTTGCTAAAATAACTTGCTTT
            GCTTGTTTATAGAGGCacagtaaatcttttttattattattataattttagattttttaatttttaaataagtgataGCA
            TAtgctgtattattaaaaatttaagaactttaaagtatttacagtagcctatattatgtaaTAGGCTAGCCTACTTTATT
            GTTCGGccaattctttttcttattcatCTTATCATTATTagcagattattatttattactagtttaaaagcacgtcaaaa
            atgacggacattaatttcttcctccttaggttgctatacttaaatgccggtcaaaaatttaaaagcccgtcaaaaataac
            agacagcgacatctatggacagaaaatagagagaaacaTCTTCGGGCaacatcggctcgccgaagatggccgagcagtat
            gacagacatcatgaccaaactttctctttattatagttagattagatagaagattagaagactagtttaaaagcccatca
            aaaatgaNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
            NNNNNNNNNNNNNNNNNgttatattgttaaacatctttacttaactctcccagaaaaaaaagttgttattcctctttata"
            ];
        // test_seqs[0]
        // 22 - 49
        // 53 - 60
        // 61 - 77
        // test_seqs[1]
        // 0 - 12
        // 23 - 50
        // 54 - 61
        // 62 - 73
        // test_seqs[2]
        // None
        // test_seqs[3]
        // 0 - 77

        for i in 0..test_seqs.len() {
            let seq = test_seqs[i].as_bytes();
            // Remove whitespace
            let seq = seq
                .iter()
                .filter(|&&x| !x.is_ascii_whitespace())
                .cloned()
                .collect::<Vec<u8>>();
            let commands = get_masking_ranges(&seq);
            let ml = convert_ranges_to_ml32bit(&commands);
            let ml_padded = pad_commands_to_u32(&ml);
            println!("{:#?}", ml_padded);
            let u32s = convert_commands_to_u32(&ml_padded);
            for i in u32s.iter() {
                println!("{:#034b}", i);
            }
            println!("Seq Length: {}", seq.len());
            println!("Length: {}", u32s.len());
            println!("Total Bits: {}", u32s.len() * 32);
            println!("Total Bytes: {}", u32s.len() * 4);

            let (num_bits, packed) = bitpack_u32(&u32s);
            println!("Num Bits: {}", num_bits);
            println!("Packed Length: {}", packed.len());
            println!("{:#?}", packed);

            let mut seq = test_seqs[i].as_bytes().to_ascii_uppercase();
            // Remove whitespace
            seq = seq
                .iter()
                .filter(|&&x| !x.is_ascii_whitespace())
                .cloned()
                .collect::<Vec<u8>>();
            // Apply commands to mask
            let commands = convert_u32_to_commands(&u32s);
            println!("{:#?}", commands);
            mask_sequence(&commands, &mut seq);

            println!("{}", test_seqs[i]);
            println!("{}", String::from_utf8(seq.clone()).unwrap());

            assert_eq!(
                seq,
                test_seqs[i]
                    .as_bytes()
                    .iter()
                    .filter(|&&x| !x.is_ascii_whitespace())
                    .cloned()
                    .collect::<Vec<u8>>()
            );
        }
    }
}
