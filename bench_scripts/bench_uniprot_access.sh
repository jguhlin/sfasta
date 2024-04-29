# wget https://ftp.uniprot.org/pub/databases/uniprot/current_release/knowledgebase/complete/uniprot_sprot.fasta.gz
# zcat uniprot_sprot.fasta.gz > uniprot_sprot.fasta
bgzip -kf uniprot_sprot.fasta
rm *.fai
samtools faidx uniprot_sprot.fasta.gz
sfa convert uniprot_sprot.fasta

hyperfine \
	--export-markdown uniprot_access.md \
	'sfa faidx uniprot_sprot.sfasta "sp|Q8CIX8|LGSN_MOUSE" "sp|O31861|YOJB_BACSU" "sp|B2XTX0|PSAJ_HETA4" "sp|A2YNP0|SPX6_ORYSI" "sp|P69474|CAPSD_CGMVS" ' \
	'samtools faidx uniprot_sprot.fasta.gz "sp|Q8CIX8|LGSN_MOUSE" "sp|O31861|YOJB_BACSU" "sp|B2XTX0|PSAJ_HETA4" "sp|A2YNP0|SPX6_ORYSI" "sp|P69474|CAPSD_CGMVS" '
