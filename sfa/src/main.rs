/*extern crate mimalloc;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;*/

extern crate bumpalo;
extern crate clap;
extern crate indicatif;
extern crate rand;
extern crate rand_chacha;

use std::fs;
use std::fs::{metadata, File};
use std::io::{BufReader, Read};
use std::path::Path;

use simdutf8::basic::from_utf8;

use bumpalo::Bump;
use clap::{load_yaml, App, ArgMatches};
use indicatif::{ProgressBar, ProgressStyle};

use libsfasta::prelude::*;

use libsfasta::CompressionType;

fn style_pb(pb: ProgressBar) -> ProgressBar {
    let style = ProgressStyle::default_bar()
        .template(
            "[{spinner:.green}] {bar:30.green/yellow} {bytes:.cyan}/{total_bytes:.blue} ({eta})",
        )
        .progress_chars("█▇▆▅▄▃▂▁  ")
        .tick_chars("ACTGN🧬");
    pb.set_style(style);
    pb
}

fn main() {
    let yaml = load_yaml!("cli.yaml");
    let matches = App::from(yaml).get_matches();

    if let Some(matches) = matches.subcommand_matches("convert") {
        convert(&matches);
    }

    if let Some(matches) = matches.subcommand_matches("stats") {
        let filename = matches.value_of("input").unwrap();
        let file = match File::open(filename) {
            Err(why) => panic!("Couldn't open {}: {}", filename, why.to_string()),
            Ok(file) => file,
        };

        let file = BufReader::with_capacity(8 * 1024 * 1024, file);
        let sfasta = SfastaParser::open_from_buffer(file);
        println!("Successfully opened SFASTA");
        println!("Found: {} entries", sfasta.index.as_ref().unwrap().len());
        println!("{:#?}", sfasta.index.as_ref().unwrap().ids);
    }

    // TODO: Make this faster but putting the decompression into another thread...
    // Probs not worth it (NT takes ~10m on my machine)
    if let Some(matches) = matches.subcommand_matches("summarize") {
        let fasta_filename = matches.value_of("input").unwrap();
        let metadata = fs::metadata(fasta_filename).expect("Unable to get filesize");
        let pb = ProgressBar::new(metadata.len());
        let pb = style_pb(pb);

        let buf = generic_open_file_pb(pb, fasta_filename);
        // let buf = pb.wrap_read(buf.2);
        let summary = summarize_fasta(&mut BufReader::with_capacity(4 * 1024 * 1024, buf.2));
        println!("File: {}", fasta_filename);
        println!("Total Entries: {}", summary.0);
        println!("Total Sequence: {}", summary.2.iter().sum::<usize>());
    }

    if let Some(matches) = matches.subcommand_matches("faidx") {
        let sfasta_filename = matches.value_of("input").unwrap();

        let in_buf = File::open(sfasta_filename).expect("Unable to open file");
        let sfasta =
            SfastaParser::open_from_buffer(BufReader::with_capacity(4 * 1024 * 1024, in_buf));

        let ids = matches.values_of("ids").unwrap();
        for i in ids {
            let results = sfasta
                .find(i)
                .expect(&format!("Unable to find {} in file {}", i, sfasta_filename))
                .unwrap();
            for result in results {
                let sequence = sfasta
                    .get_sequence(&result.3)
                    .expect("Unable to fetch sequence");
                println!(">{}", i);
                println!("{}", from_utf8(&sequence).unwrap());
            }
        }
    }

    // TODO: Store sequence IDs in insertion order at the end of the file
    // insertion-order output should be a custom function
    // (Decompress entire block, output sequences appropriately)
    // So don't need this more random (and slower) method anymore...
    if let Some(matches) = matches.subcommand_matches("view") {
        // TODO: Add ability to store sequences in order, to reorder sequences, and
        // to identify when sequences are in order and display them that way...

        let sfasta_filename = matches.value_of("input").unwrap();

        let in_buf = File::open(sfasta_filename).expect("Unable to open file");
        let mut sfasta =
            SfastaParser::open_from_buffer(BufReader::with_capacity(8 * 1024 * 1024, in_buf));

        // TODO: We could process this in blocks to be more memory efficient...
        sfasta.decompress_all_ids();

        let mut bump = Bump::new();

        for seqid in sfasta.index.as_ref().unwrap().ids.as_ref().unwrap() {
            let results = sfasta
                .find(&seqid)
                .expect(&format!("Unable to find {} in file {}, even though it is in the index! File is likely corrupt, or this is a serious bug.", &seqid, sfasta_filename))
                .unwrap();

            // TODO: Disabled bumpalo... for now...
            for result in results {
                let sequence = sfasta
                    .get_sequence(&result.3)
                    .expect("Unable to fetch sequence");
                println!(">{}", seqid);
                println!("{}", from_utf8(&sequence).unwrap());
            }

            bump.reset();
        }
    }

    if let Some(matches) = matches.subcommand_matches("list") {
        let sfasta_filename = matches.value_of("input").unwrap();

        let in_buf = File::open(sfasta_filename).expect("Unable to open file");
        let mut sfasta =
            SfastaParser::open_from_buffer(BufReader::with_capacity(32 * 1024 * 1024, in_buf));

        sfasta.decompress_all_ids();

        for i in sfasta.index.as_ref().unwrap().ids.as_ref().unwrap().iter() {
            println!("{}", i);
        }
    }

    /*
    if let Some(matches) = matches.subcommand_matches("stats") {
        stats(&matches);
    }
    if let Some(matches) = matches.subcommand_matches("split") {
        split(&matches);
    }
    if let Some(matches) = matches.subcommand_matches("index") {
        index_file(&matches);
    } */
}

// TODO: Set metadata
// TODO: Set masking option
// TODO: Block sizes, index compression type, etc...
fn convert(matches: &ArgMatches) {
    let fasta_filename = matches.value_of("input").unwrap();
    let threads: usize = matches.value_of_t("threads").unwrap_or(4);

    let metadata = fs::metadata(fasta_filename).expect("Unable to get filesize");
    let pb = ProgressBar::new(metadata.len());
    let pb = style_pb(pb);

    let buf = generic_open_file_pb(pb, fasta_filename);
    // let buf = pb.wrap_read(buf.2);
    /*let summary = count_fasta_entries(&mut BufReader::with_capacity(32 * 1024 * 1024, buf.2));
    println!("File: {}", fasta_filename);
    println!("Total Entries: {}", summary); */

    let path = Path::new(fasta_filename);
    let output_name = path.with_extension("sfasta");
    let mut output = match File::create(output_name) {
        Err(why) => panic!("couldn't create: {}", why),
        Ok(file) => file,
    };

    let metadata = fs::metadata(fasta_filename).expect("Unable to get filesize");
    let pb = ProgressBar::new(metadata.len());
    let pb = style_pb(pb);

    let buf = generic_open_file_pb(pb, fasta_filename);
    // let buf = BufReader::with_capacity(2 * 1024 * 1024, buf.2);
    let buf = buf.2;

    // Disabled for now: No improvement
    // let dict = build_dict(buf);

    let metadata = fs::metadata(fasta_filename).expect("Unable to get filesize");
    let pb = ProgressBar::new(metadata.len());
    let pb = style_pb(pb);

    let buf = generic_open_file_pb(pb, fasta_filename);
    // let buf = BufReader::with_capacity(2 * 1024 * 1024, buf.2);
    let buf = buf.2;

    let mut compression_type = CompressionType::default();
    if matches.is_present("zstd") {
        compression_type = CompressionType::ZSTD;
    } else if matches.is_present("lz4") {
        compression_type = CompressionType::LZ4;
    } else if matches.is_present("xz") {
        compression_type = CompressionType::XZ;
    } else if matches.is_present("brotli") {
        compression_type = CompressionType::BROTLI;
    } else if matches.is_present("gzip") {
        println!("🤨");
        compression_type = CompressionType::GZIP;
    }

    let mut converter = Converter::default()
        .with_threads(threads)
        .with_compression_type(compression_type);

    // Disabled for now, no improvement
    // converter = converter.with_dict(dict);

    if matches.is_present("block_size") {
        let block_size: usize = matches.value_of_t("block_size").unwrap();
        converter = converter.with_block_size(block_size * 1024);
    }

    if matches.is_present("index_chunk_size") {
        let chunk_size: usize = matches.value_of_t("index_chunk_size").unwrap();
        converter = converter.with_index_chunk_size(chunk_size);
    }

    if matches.is_present("seqlocs_chunk_size") {
        let chunk_size: usize = matches
            .value_of_t("seqlocs_chunk_size")
            .unwrap_or(64 * 1024);
        converter = converter.with_seqlocs_chunk_size(chunk_size);
    }

    if matches.is_present("noindex") {
        println!("Noindex received");
        converter = converter.without_index();
    }

    converter.convert_fasta(buf, &mut output);
}

pub fn generic_open_file_pb(
    pb: ProgressBar,
    filename: &str,
) -> (usize, bool, Box<dyn Read + Send>) {
    let filesize = metadata(filename)
        .unwrap_or_else(|_| panic!("{}", &format!("Unable to open file: {}", filename)))
        .len();

    let file = match File::open(filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why.to_string()),
        Ok(file) => file,
    };

    let mut compressed: bool = false;
    let file = pb.wrap_read(file);

    let fasta: Box<dyn Read + Send> = if filename.ends_with("gz") {
        compressed = true;
        Box::new(flate2::read::GzDecoder::new(file))
    } else if filename.ends_with("snappy") || filename.ends_with("sz") || filename.ends_with("sfai")
    {
        compressed = true;
        Box::new(snap::read::FrameDecoder::new(file))
    } else {
        Box::new(file)
    };

    (filesize as usize, compressed, fasta)
}
