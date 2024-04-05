# Short Term
- Struct's should handle encoding / decoding, so if they require fixed int encoding it's always that way (rather than remembering it elsewhere)
* Output block should have an "is_raw" flag to avoid double bincoding
- Double bincoding is both a serious problem (esp. switching between variable and fixed int encoding) and necessary for compressed blocks
- Block locs should be fractaltree as well u32, u64 so that we don't have to decompress all of them!
- Reenable mimalloc

## SFA 
* sfa - Switch to needletail instead of custom made solution 

# Long Term

* Pin compression workers to CPUs
* Use syncmers to group together similar sequences ( Only useful for very large blocks though )
* Use Pulp to speed up on certain platforms
* Custom errors