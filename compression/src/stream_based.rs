// This is likely a separate project...

// This is modified from a paper (presentation? white paper?)
// INSPO: https://saurabhkadekodi.github.io/dna_compression.pdf
// from another person, but I am offline with no internet right now...
// must find that for citation and to see if anything was missed

// Overall guide
// Homopolymer compress - Store in intermediate format
// Kmer dictionary creation, find all kmers (not HPC compressed
// regions) All kmers of 9, 11, 13, 15, 17, 19, 21
// Find occurences and figure out best savings
// Kmers can repeat like HPC stuff too with commands
// Convert to bits -> hpc gets converted to repeats, kmers get
// converted to appropriate dictionary
//
// IDEA: Make 2 dictionaries for >= 50% GC and < 50% GC
// Maybe even more refined.... but depends on the data? Not always
// worth a huge dictionary Could do for different GC contents (for
// esp. large datasets, like nt)

// Split ACTGN into individual streams
// A C T G N
// A - [1 0 0 0 0]
// C - [0 1 0 0 0]
// T - [0 0 1 0 0]
// G - [0 0 0 1 0]
// N - [0 0 0 0 0] (N stream is actually "hidden") and N's are
// represented by 0's across the whole column Instead the 5th stream
// is a line of integers used per event (described later)

// Actions are coded into the columns
//
// Homopolymer-style Repeat
// A - [1 0 1 0 1 1 0 0]
// T - [0 1 1 1 0 1 1 1]
// C - [0 0 1 0 0 1 0 0]
// G - [0 0 1 0 0 1 0 0]
// Control / Integer Channel
// CC - [4]
// So the above sequence is ATTATATATATATT
// The A and T as specified in the normal presentation
// Column of all 1's to represent start a repeat, of TA
// Column of all 1's to represent ending of a repeat
// So now the sequence is ATTA
// Now add the 4 additional repeats - ATTATATATATA
// (The original repeat is also expressed, so the total length is
// control channel integer + 1) (This is up for debate, but seems
// better to store smaller numbers for compression) Then TT follows

// Control channel is stream vbyte compressed

// Nucleotide signals are then encoded into bits with RLE
// A - [1 0 1 0 1 1 0 0]
// T - [0 1 1 1 0 1 1 1]
// C - [0 0 1 0 0 1 0 0]
// G - [0 0 1 0 0 1 0 0]
// Initial bit, 00 for A, 01 for T, 10 for C, 11 for G
// Then the A channel follows, T, C, G, then control channel
// Initial bit is 00 for A
// RLE for A is encoded as a series of u4's, with 0 and 1 alternating
// for each number, but flipped for the A channel due to the initial
// bit, which is encoded as alternative 1's and 0's Channel A becomes
// [1 1 1 1 2 2] (as u4's, so 6 * 4, total of 3 bytes)
// Channel T becomes [1 3 1 3] (4 * 4 bits, so 2 bytes)
// Channel C becomes [2 1 2 1 2] (5 * 4 bits, 2.5 bytes)
// Channel G becomes [2 1 2 1 2] (5 * 4 bits, 2.5 bytes)
// Total of 10 bytes to store ATTATATATATATT (14 bytes)
// so not a great compression ratio, but this is an example set
// u4's go up to 16, so u5 or u6 may be more appropriate
// IDEA: programmatically determine u3, u4, u5, u6, u7, u8 (shouldn't
// be higher, but good to test) and store as early bits in the
// stream..

// Other idea, 2-bit RLE
// A - [1 0 1 0 1 1 0 0] -> [1 0 2_u4 1 1 1_u4 0 0 1_u4], bit bit u4,
// bit bit u4, bit bit u4 -> 2.5 bytes T - [0 1 1 1 0 1 1 1] -> [0 1
// 1_u4 1 1 1_u4 0 1 1_u4 1 1 1_u4] 3 bytes C - [0 0 1 0 0 1 0 0] ->
// [0 0 1_u4 1 0 1_u4 0 1 1_u4 0 0 1_u4] 3 bytes G - [0 0 1 0 0 1 0 0]
// -> [0 0 1_u4 1 0 1_u4 0 1 1_u4 0 0 1_u4] 3 bytes Total of 11.5
// bytes (1.5 bytes more than 1-bit RLE) But maybe worth testing on
// real data, maybe worth having both as an option This would catch
// TATATATATA but so does homopolymer compression

// For stretches that go longer, set the next to 0 and then continue.
// easy! There may be other patterns / 2 bit RLE or something that
// works better

// Other "commands via columns"
// need to benchmark and see if too slow
// A - [1]
// T - [1]
// C - [1]
// G - [0]
// Any combination with more than a single 1 but less than 4 counts as
// a dictionary reference, with the next integer in the control
// channel the reference So this supports X possible dictionaries
// (ACTG channel across) - [1 0 0 1] [1 0 1 0] [1 0 1 1] [1 1 0 0] [1
// 1 0 1] [1 1 1 0]                         [0 0 1 1] [0 1 0 1] [0 1 1
// 0] So 10 combinations, support for up to 9 dicts, with u8 (or u16)
// as dict size Allows a very large dictionary, and multiple
// references Also could support different dictionaries for >= 50% GC
// <= 50% GC and set that with a bit switch (double the dict capacity)

// Other ideas below...
// A 2nd round of RLE does the following:
// Channel A [1 1 1 1 2 2] -> [1 4 2 2] (as u4's, so 2 bytes)
// Channel T [1 3 1 3]     -> [1 1 3 1 1 1 3 1] (8 u4's, so 4 bytes)
// Channel C [2 1 2 1 2]   -> [2 1 1 1 2 1 1 1 2 1] (10 u4's, 5 bytes)
// Channel G [2 1 2 1 2]   -> [2 1 1 1 2 1 1 1 2 1] (10 u4's, 5 bytes)
// So double RLE did not work in this case -- but for a real example?
// Maybe, or some other alternative

// Because I'm offline! Can't get the crate
type u4 = u8;

struct DictEntry(u4, u8);

enum IntermediateRepr
{
    A,
    C,
    T,
    G,
    HPC(Vec<IntermediateRepr>, u8), // Homopolymer compressed
    Dict(DictEntry),                // Dictionary to use, kmer to use
    HPCDict(DictEntry),
}
