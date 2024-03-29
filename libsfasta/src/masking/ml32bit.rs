use bitvec::prelude::*;
use rayon::prelude::*;

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
    #[inline]
    pub fn bitsize(&self) -> usize {
        match self {
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
            Ml32bit::Pass => 4,
            Ml32bit::Stop => 4,
        }
    }

    pub const fn important_bits(&self) -> &'static str {
        match self {
            Ml32bit::SkipAheadu4(_) => "1000____",
            Ml32bit::SkipAheadu8(_) => "0100________",
            Ml32bit::SkipAheadu16(_) => "0101________________",
            Ml32bit::SkipAheadu20(_) => "0010____________________",
            Ml32bit::SkipAheadu24(_) => "0011________________________",
            Ml32bit::Masku4(_) => "1001____",
            Ml32bit::Masku8(_) => "1010________",
            Ml32bit::Masku16(_) => "1011________________",
            Ml32bit::Masku20(_) => "1100____________________",
            Ml32bit::Masku24(_) => "1101________________________",
            Ml32bit::Pass => "0000",
            Ml32bit::Stop => "1111",
        }
    }
}

#[allow(non_snake_case)]
pub const fn Ml32bitPossibilities() -> [Ml32bit; 12] {
    [
        Ml32bit::Pass,
        Ml32bit::SkipAheadu4(0),
        Ml32bit::SkipAheadu8(0),
        Ml32bit::SkipAheadu16(0),
        Ml32bit::SkipAheadu20(0),
        Ml32bit::SkipAheadu24(0),
        Ml32bit::Masku4(0),
        Ml32bit::Masku8(0),
        Ml32bit::Masku16(0),
        Ml32bit::Masku20(0),
        Ml32bit::Masku24(0),
        Ml32bit::Stop,
    ]
}

// TODO: Could we match all combinations by using a const fn to generate a lookup table?
// And would it be faster?
pub fn _parse_ml32bit(ml: u32) -> Vec<Ml32bit> {
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
        let idx = if is_masked {
            seq[pos..].iter().position(|x| x.is_ascii_uppercase())
        } else {
            seq[pos..].iter().position(|x| x.is_ascii_lowercase())
        };

        if let Some(x) = idx {
            if is_masked {
                ranges.push((pos, pos.wrapping_add(x)));
            }
            is_masked = !is_masked;
            pos = pos.wrapping_add(x);
        } else {
            break;
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

#[inline]
pub fn convert_commands_to_u32(commands: &[Ml32bit]) -> Vec<u32> {
    let mut u32s = Vec::with_capacity(commands.len());
    let mut i = 0;
    let mut cur_u32 = 0u32;
    let mut cur_u32_bits = cur_u32.view_bits_mut::<Lsb0>();

    for command in commands {
        assert!(i <= 28);
        match command {
            Ml32bit::SkipAheadu24(len) => {
                cur_u32_bits[i..i + 4].store(0b0111);
                cur_u32_bits[i + 4..i + 28].store(*len as u32);
                i += 28;
            }
            Ml32bit::SkipAheadu20(len) => {
                cur_u32_bits[i..i + 4].store(0b0110);
                cur_u32_bits[i + 4..i + 24].store(*len as u32);
                i += 24;
            }
            Ml32bit::SkipAheadu16(len) => {
                cur_u32_bits[i..i + 4].store(0b0101);
                cur_u32_bits[i + 4..i + 20].store(*len as u16);
                i += 20;
            }
            Ml32bit::SkipAheadu8(len) => {
                cur_u32_bits[i..i + 4].store(0b0100);
                cur_u32_bits[i + 4..i + 12].store(*len as u8);
                i += 12;
            }
            Ml32bit::SkipAheadu4(len) => {
                cur_u32_bits[i..i + 4].store(0b1000);
                cur_u32_bits[i + 4..i + 8].store(*len);
                i += 8;
            }
            Ml32bit::Masku24(len) => {
                cur_u32_bits[i..i + 4].store(0b1001);
                cur_u32_bits[i + 4..i + 28].store(*len as u32);
                i += 28;
            }
            Ml32bit::Masku20(len) => {
                cur_u32_bits[i..i + 4].store(0b1000);
                cur_u32_bits[i + 4..i + 24].store(*len as u32);
                i += 24;
            }
            Ml32bit::Masku16(len) => {
                cur_u32_bits[i..i + 4].store(0b0111);
                cur_u32_bits[i + 4..i + 20].store(*len as u16);
                i += 20;
            }
            Ml32bit::Masku8(len) => {
                cur_u32_bits[i..i + 4].store(0b0110);
                cur_u32_bits[i + 4..i + 12].store(*len as u8);
                i += 12;
            }
            Ml32bit::Masku4(len) => {
                cur_u32_bits[i..i + 4].store(0b1001);
                cur_u32_bits[i + 4..i + 8].store(*len);
                i += 8;
            }
            Ml32bit::Stop => {
                cur_u32_bits[i..i + 4].store(0b1111);
                u32s.push(cur_u32);
                cur_u32 = 0u32;
                cur_u32_bits = cur_u32.view_bits_mut::<Lsb0>();
                i = 0;
            }
            Ml32bit::Pass => {
                cur_u32_bits[i..i + 4].store(0b0000);
                u32s.push(cur_u32);
                cur_u32 = 0u32;
                cur_u32_bits = cur_u32.view_bits_mut::<Lsb0>();
                i = 0;
            }
        }

        if i == 32 {
            u32s.push(cur_u32);
            cur_u32 = 0u32;
            cur_u32_bits = cur_u32.view_bits_mut::<Lsb0>();
            i = 0;
        } else if i > 32 {
            // This doesn't happen because we've already done padding
            panic!("i > 32");
        }
    }

    if i > 0 {
        u32s.push(cur_u32);
    }

    u32s
}

// Faster, but at a cost of file size
#[allow(dead_code)]
#[inline]
pub fn convert_commands_to_u32_uncompressed(commands: &[Ml32bit]) -> Vec<u32> {
    let mut u32s = Vec::with_capacity(commands.len());
    let mut cur_u32 = 0u32;

    for command in commands {
        match command {
            Ml32bit::SkipAheadu24(len) => {
                cur_u32 = cur_u32 | 0b0111 << 28;
                cur_u32 += *len as u32;
            }
            Ml32bit::SkipAheadu20(len) => {
                cur_u32 = cur_u32 | 0b0111_0000_0000_0000_0000_0000_0000_0000;
                cur_u32 += *len as u32;
            }
            Ml32bit::SkipAheadu16(len) => {
                cur_u32 = cur_u32 | 0b0111_0000_0000_0000_0000_0000_0000_0000;
                cur_u32 += *len as u32;
            }
            Ml32bit::SkipAheadu8(len) => {
                cur_u32 = cur_u32 | 0b0111_0000_0000_0000_0000_0000_0000_0000;
                cur_u32 += *len as u32;
            }
            Ml32bit::SkipAheadu4(len) => {
                cur_u32 = cur_u32 | 0b0111_0000_0000_0000_0000_0000_0000_0000;
                cur_u32 += *len as u32;
            }
            Ml32bit::Masku24(len) => {
                cur_u32 = cur_u32 | 0b1001_0000_0000_0000_0000_0000_0000_0000;
                cur_u32 += *len as u32;
            }
            Ml32bit::Masku20(len) => {
                cur_u32 = cur_u32 | 0b1001_0000_0000_0000_0000_0000_0000_0000;
                cur_u32 += *len as u32;
            }
            Ml32bit::Masku16(len) => {
                cur_u32 = cur_u32 | 0b1001_0000_0000_0000_0000_0000_0000_0000;
                cur_u32 += *len as u32;
            }
            Ml32bit::Masku8(len) => {
                cur_u32 = cur_u32 | 0b1001_0000_0000_0000_0000_0000_0000_0000;
                cur_u32 += *len as u32;
            }
            Ml32bit::Masku4(len) => {
                cur_u32 = cur_u32 | 0b1001_0000_0000_0000_0000_0000_0000_0000;
                cur_u32 += *len as u32;
            }
            Ml32bit::Stop => {
                cur_u32 = cur_u32 | 0b1111_0000_0000_0000_0000_0000_0000_0000;
            }
            Ml32bit::Pass => {
                cur_u32 = cur_u32 | 0b0000_0000_0000_0000_0000_0000_0000_0000;
            }
        }

        u32s.push(cur_u32);
        cur_u32 = 0u32;
    }

    u32s
}

// From: https://play.rust-lang.org/?version=beta&mode=release&edition=2018&gist=2ff849086024a0a01b958060c3434570
#[allow(dead_code)]
struct BitPattern {
    expected: u32,
    mask: u32,
}

#[allow(dead_code)]
impl BitPattern {
    /// Accepts a bit pattern as a string literal.
    /// - '0' matches a 0 bit
    /// - '1' matches a 1 bit
    /// - any other char means "ignore this bit"
    const fn new(s: &str) -> Self {
        let mut expected = 0;
        let mut mask = !0;

        let mut cur_bit = 1 << (s.len() - 1);

        let mut i = 0;
        while i < s.len() {
            let val: u8 = s.as_bytes()[i];
            i += 1;

            if val == b'1' {
                expected |= cur_bit;
            } else if val == b'0' {
                // do nothing
            } else {
                mask &= !cur_bit;
            }

            cur_bit >>= 1;
        }

        Self { expected, mask }
    }
    const fn matches(&self, val: u32) -> bool {
        (val & self.mask) == self.expected
    }
}

// Probably the worst way to get a speedup....
pub fn convert_u32_to_commands2(u32s: &[u32]) -> Vec<Ml32bit> {
    u32s.par_iter()
        .map(|cur_u32| {
            let cur_u32_bits = cur_u32.view_bits::<Lsb0>();
            let mut i = 0;
            let mut commands = Vec::new();
            while i < 32 {
                let command = match cur_u32_bits[i..i + 4].load::<u8>() {
                    0b0000 => {
                        // PASS command indicates skip to next u32
                        i = 32;
                        continue;
                    }
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
            commands
        })
        .collect::<Vec<Vec<Ml32bit>>>()
        .iter()
        .flatten()
        .cloned()
        .collect()
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
                    // PASS command indicates skip to next u32
                    i = 32;
                    continue;
                }
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
            let u32s = convert_commands_to_u32(&ml_padded);
            for i in u32s.iter() {
                println!("{}, {:#034b}, {}", i, i, (i & 4) == 4);
            }

            println!("Seq Length: {}", seq.len());
            println!("Length: {}", u32s.len());
            println!("Total Bits: {}", u32s.len() * 32);
            println!("Total Bytes: {}", u32s.len() * 4);

            let (num_bits, packed) = bitpack_u32(&u32s);
            println!("Num Bits: {}", num_bits);
            println!("Packed Length: {}", packed.len());

            let mut seq = test_seqs[i].as_bytes().to_ascii_uppercase();
            // Remove whitespace
            seq = seq
                .iter()
                .filter(|&&x| !x.is_ascii_whitespace())
                .cloned()
                .collect::<Vec<u8>>();
            // Apply commands to mask
            let commands = convert_u32_to_commands(&u32s);
            mask_sequence(&commands, &mut seq);

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
