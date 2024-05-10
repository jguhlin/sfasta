# wget https://ftp.uniprot.org/pub/databases/uniprot/current_release/knowledgebase/complete/uniprot_sprot.fasta.gz
# zcat uniprot_sprot.fasta.gz > uniprot_sprot.fasta
hyperfine \
	--export-markdown uniprot_conversion.md \
	--export-csv "uniprot_conversion.csv" \
	'sfa convert --threads 14 uniprot_sprot.fasta' \
	'bgzip -kf --threads 16 uniprot_sprot.fasta' \
	'pigz -kf -p 16 uniprot_sprot.fasta' \
	'ennaf --protein --temp-dir /tmp uniprot_sprot.fasta -o uniprot_sprot.naf' \
	'zstd -k uniprot_sprot.fasta -f -T16' \
	'crabz -f bgzf -p 16 uniprot_sprot.fasta -o uniprot_sprot.fasta.gz'
