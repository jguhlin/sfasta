// This is a derivation of the SortedVec tree (fastest, so far)
// Fractal adds a buffer so that insertions etc... are done in batches, rather than immediately
//
// NOTE: Optimized for very large tree is order 128, buffer_size 256
//
// Todo:
// - Storable on disk
// - Able to load only part of the tree from disk
// - Specialized impl for <u64, u32> as that's what we use in the sfasta format,
//   then allow simd accel? Using wide or pulp?

// ----------------------------------------------
// This is where I'm at right now:
//   - Get the on disk tree working efficiently
//   - Then the SeqLocs stuff working (load up needed seqloc data, but for the most part
//     keep it on disk)
//   - Have option to load all seqlocs into memory for random access based on index though!
// ----------------------------------------------

// Specifically
// - NodeOnDisk will attempt to load from the reader if it's not in memory
// - FractalTreeDisk will have a start position, and will load the root node from the reader
// - Search function with automatic loading into memory
// NEXT STEPS:
// - For node encode, try Bitpacking
// - Implement loading to disk (start from the bottom up)

// Dev Log
// - Better storage of NodeDisk
// - Split into separate modules, use a trait so we can use dyn and
// have FractalTreeRead and FractalTreeDisk (Can't do dyn, will have to do enum or something)
// - Compression
// - Tried out VBytes and varint for encoding nodes, as well as Zstd
// - Sticking with Vbytes, even though compression is small, as it should be faster to parse
//
// Notes:
// [x] Look into eytzinger (instead of sorted vec?) or ordsearch?
//     -- Neither worked well here
// I think FractalTreeRead is redundant