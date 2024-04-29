hyperfine \
	--export-markdown illumina_conversion.md \
	--export-csv "illumina_conversion.csv" \
	'sfa convert --threads 14 reads.fastq' \
	'bgzip -kf --threads 16 reads.fastq' \
	'pigz -kf -p 16 reads.fastq' \
	'ennaf --dna --fastq --temp-dir /tmp reads.fastq -o reads.naf' \
	'zstd -k reads.fastq -f -T16' \
	'crabz -f bgzf -p 16 reads.fastq -o reads.fastq.gz'
