rm bench_sizes.txt
hyperfine -L compressor zstd,lz4,brotli,xz './sfa convert reads.fasta --{compressor} --index' --export-csv bench.csv --export-markdown bench.md -c 'stat --format="%s" reads.sfasta >> bench_sizes.txt'
