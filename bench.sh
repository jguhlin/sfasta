rm bench_sizes.txt
hyperfine -L compressor zstd,lz4,brotli,xz './sfa convert reads.fasta --{compressor} --index' --export-csv bench.csv --export-markdown bench.md -c 'stat --format="%s" reads.sfasta >> bench_sizes.txt'
hyperfine 'gzip reads.fasta' 'pigz reads.fasta' --export-csv gzip.csv --export-markdown gzip.md -c 'stat --format="%s" reads.fasta.gz >> bench_gzip_sizes.txt'
