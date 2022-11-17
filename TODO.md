* Pin compression workers to CPUs
* Use syncmers to group together similar sequences ( Only useful for very large blocks though )
* Use Pulp to speed up on certain platforms
* Magic chunkify struct
* All prefetch functions should coerce a BufReader